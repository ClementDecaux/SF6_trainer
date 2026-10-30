//! Le son du jeu dans les vidéos.
//!
//! Windows (10 version 2004 et plus) sait « écouter » un seul programme : WASAPI en mode
//! loopback de processus. On n'entend donc que SF6, pas la musique ni Discord. Un thread
//! le fait en continu et garde les dernières secondes en mémoire, en PCM 16 bits stéréo, avec
//! l'heure de chaque échantillon sur la même horloge que les images de la capture. Quand une
//! vidéo commence, la capture prend le son à partir de l'heure de sa 1re image : il est calé
//! sur les images, même pour celles filmées avant la 1re étape.
//!
//! Quand rien ne joue, Windows n'envoie rien : on comble avec du silence pour que le son
//! reste à l'heure.

// Hors Windows, pas de son : une partie du code n'y sert que pour les tests.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::VecDeque;
use std::sync::Mutex;

/// Unités de 100 ns par seconde (l'horloge des images et du son).
const SECONDE: i128 = 10_000_000;

/// Ce qu'on garde du son : plus que les images d'avant la 1re étape, pour avoir de la marge.
const MARGE_MS: u64 = 5_000;

/// Les dernières secondes du son, en trames stéréo (gauche, droite).
pub struct TamponSon {
    /// Échantillons par seconde.
    pub taux: u32,
    /// Heure de la trame numéro 0 (la toute première reçue).
    origine: i64,
    /// Trames jetées depuis le début : la 1re trame gardée porte ce numéro.
    retirees: u64,
    trames: VecDeque<[i16; 2]>,
    garder: usize,
}

impl TamponSon {
    pub fn new(taux: u32, garder_ms: u64) -> TamponSon {
        TamponSon {
            taux,
            origine: 0,
            retirees: 0,
            trames: VecDeque::new(),
            garder: (garder_ms * u64::from(taux) / 1000) as usize,
        }
    }

    /// Numéro de la trame jouée à l'heure `temps` (`None` tant que rien n'est reçu).
    pub fn numero(&self, temps: i64) -> Option<u64> {
        if self.retirees == 0 && self.trames.is_empty() {
            return None;
        }
        let ecart = i128::from(temps - self.origine).max(0);
        Some((ecart * i128::from(self.taux) / SECONDE) as u64)
    }

    /// Numéro de la trame qui suit la dernière reçue.
    fn fin(&self) -> u64 {
        self.retirees + self.trames.len() as u64
    }

    /// Des trames reçues de Windows, la 1re jouée à l'heure `temps`.
    pub fn ajouter(&mut self, temps: i64, trames: &[[i16; 2]]) {
        let Some(numero) = self.numero(temps) else {
            self.origine = temps;
            self.trames.extend(trames);
            self.jeter_les_vieilles();
            return;
        };
        let fin = self.fin();
        // Un petit décalage (moins de 10 ms) vient des arrondis : on colle à la suite.
        let tolerance = u64::from(self.taux / 100);
        let mut debut = 0;
        if numero > fin + tolerance {
            // Un trou (rien ne jouait) : du silence jusqu'à ces trames.
            self.silence((numero - fin) as usize);
        } else if numero + tolerance < fin {
            // Déjà comblé par du silence : on ne garde que la suite.
            debut = ((fin - numero) as usize).min(trames.len());
        }
        self.trames.extend(&trames[debut..]);
        self.jeter_les_vieilles();
    }

    /// Rien reçu jusqu'à l'heure `temps` : c'était du silence.
    pub fn silence_jusqua(&mut self, temps: i64) {
        if let Some(numero) = self.numero(temps)
            && numero > self.fin()
        {
            self.silence((numero - self.fin()) as usize);
            self.jeter_les_vieilles();
        }
    }

    fn silence(&mut self, trames: usize) {
        self.trames.extend(std::iter::repeat_n([0, 0], trames));
    }

    fn jeter_les_vieilles(&mut self) {
        if self.trames.len() > self.garder {
            let trop = self.trames.len() - self.garder;
            self.trames.drain(..trop);
            self.retirees += trop as u64;
        }
    }

    /// Le son à envoyer à l'encodeur, de la trame `depuis` jusqu'à l'heure `jusqua` (pas
    /// plus loin que ce qui est reçu), en PCM 16 bits stéréo. Renvoie aussi le numéro de la
    /// trame suivante. Ce qui a déjà été jeté du tampon est remplacé par du silence.
    pub fn lire(&self, depuis: u64, jusqua: i64) -> (Vec<u8>, u64) {
        let Some(limite) = self.numero(jusqua) else {
            return (Vec::new(), depuis);
        };
        let fin = limite.min(self.fin()).max(depuis);
        let mut octets = Vec::with_capacity(((fin - depuis) * 4) as usize);
        for n in depuis..fin {
            let trame = n
                .checked_sub(self.retirees)
                .and_then(|i| self.trames.get(i as usize))
                .copied()
                .unwrap_or([0, 0]);
            for echantillon in trame {
                octets.extend_from_slice(&echantillon.to_le_bytes());
            }
        }
        (octets, fin)
    }
}

/// Le son écouté en continu (rempli par le thread WASAPI).
static SON: Mutex<Option<TamponSon>> = Mutex::new(None);

/// Échantillons par seconde du son écouté (`None` : pas de son).
pub fn taux() -> Option<u32> {
    SON.lock().ok()?.as_ref().map(|s| s.taux)
}

/// Numéro de la trame jouée à l'heure `temps`.
pub fn numero(temps: i64) -> Option<u64> {
    SON.lock().ok()?.as_ref()?.numero(temps)
}

/// Voir [`TamponSon::lire`].
pub fn lire(depuis: u64, jusqua: i64) -> (Vec<u8>, u64) {
    match SON.lock().ok().as_ref().and_then(|s| s.as_ref()) {
        Some(son) => son.lire(depuis, jusqua),
        None => (Vec::new(), depuis),
    }
}

/// Commence à écouter le son de SF6 (`processus`, le programme du jeu), et seulement lui.
/// Sans effet hors Windows.
pub fn demarrer(processus: &str, avant_ms: u64) {
    #[cfg(windows)]
    {
        let processus = processus.to_string();
        std::thread::spawn(move || wasapi::boucle(processus, avant_ms + MARGE_MS));
    }
    #[cfg(not(windows))]
    let _ = (processus, avant_ms);
}

#[cfg(windows)]
mod wasapi {
    use super::{SON, TamponSon};
    use std::sync::{Mutex, mpsc};
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Media::Audio::{
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
        AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
        AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync,
        IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
        IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
        PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        WAVEFORMATEX,
    };
    use windows::Win32::System::Com::StructuredStorage::{
        PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
    };
    use windows::Win32::System::Com::{BLOB, COINIT_MULTITHREADED, CoInitializeEx};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::Variant::VT_BLOB;
    use windows::core::{IUnknown, Interface, Ref, implement};

    type Erreur = Box<dyn std::error::Error>;

    /// Le son demandé à Windows : PCM 16 bits stéréo à 48 kHz (il convertit lui-même).
    const TAUX: u32 = 48_000;

    /// Attend SF6, écoute son son tant qu'il tourne, et recommence.
    pub fn boucle(processus: String, garder_ms: u64) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut prevenu = false;
        loop {
            if let Some(pid) = trouver_processus(&processus) {
                match ecouter(pid, &processus, garder_ms) {
                    Ok(()) => prevenu = false,
                    Err(e) if !prevenu => {
                        eprintln!(
                            "Son : impossible d'écouter le son de SF6 ({e}). Vidéos sans son \
                             (il faut Windows 10 version 2004 ou plus récent)."
                        );
                        prevenu = true;
                    }
                    Err(_) => {}
                }
            }
            if let Ok(mut son) = SON.lock() {
                *son = None;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    }

    /// Le numéro du processus de SF6 (`processus` dans capture.toml).
    fn trouver_processus(nom: &str) -> Option<u32> {
        unsafe {
            let liste = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            let mut entree = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut trouve = None;
            let mut suite = Process32FirstW(liste, &mut entree).is_ok();
            while suite {
                let longueur = entree
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entree.szExeFile.len());
                let exe = String::from_utf16_lossy(&entree.szExeFile[..longueur]);
                if exe.eq_ignore_ascii_case(nom) {
                    trouve = Some(entree.th32ProcessID);
                    break;
                }
                suite = Process32NextW(liste, &mut entree).is_ok();
            }
            let _ = CloseHandle(liste);
            trouve
        }
    }

    /// Prévient quand Windows a fini de préparer l'écoute.
    #[implement(IActivateAudioInterfaceCompletionHandler)]
    struct Fini(Mutex<Option<mpsc::Sender<()>>>);

    impl IActivateAudioInterfaceCompletionHandler_Impl for Fini_Impl {
        fn ActivateCompleted(
            &self,
            _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
        ) -> windows::core::Result<()> {
            if let Ok(mut envoi) = self.0.lock()
                && let Some(envoi) = envoi.take()
            {
                let _ = envoi.send(());
            }
            Ok(())
        }
    }

    /// Un client audio qui n'entend que le processus `pid` (et ceux qu'il lance).
    fn client_du_processus(pid: u32) -> Result<IAudioClient, Erreur> {
        unsafe {
            let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
                ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
                Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                    ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                        TargetProcessId: pid,
                        ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                    },
                },
            };
            // Les paramètres passent dans un PROPVARIANT de type BLOB qui pointe dessus
            // (pas de PropVariantClear : la mémoire est à nous).
            let variante = std::mem::ManuallyDrop::new(PROPVARIANT {
                Anonymous: PROPVARIANT_0 {
                    Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                        vt: VT_BLOB,
                        wReserved1: 0,
                        wReserved2: 0,
                        wReserved3: 0,
                        Anonymous: PROPVARIANT_0_0_0 {
                            blob: BLOB {
                                cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                                pBlobData: &mut params as *mut _ as *mut u8,
                            },
                        },
                    }),
                },
            });
            let (envoi, reception) = mpsc::channel();
            let fini: IActivateAudioInterfaceCompletionHandler =
                Fini(Mutex::new(Some(envoi))).into();
            let operation = ActivateAudioInterfaceAsync(
                VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
                &IAudioClient::IID,
                Some(&*variante as *const PROPVARIANT),
                &fini,
            )?;
            reception
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| "Windows ne répond pas")?;
            let mut resultat = windows::core::HRESULT(0);
            let mut interface: Option<IUnknown> = None;
            operation.GetActivateResult(&mut resultat, &mut interface)?;
            resultat.ok()?;
            Ok(interface.ok_or("pas de client audio")?.cast()?)
        }
    }

    fn ecouter(pid: u32, processus: &str, garder_ms: u64) -> Result<(), Erreur> {
        unsafe {
            let client = client_du_processus(pid)?;
            let format = WAVEFORMATEX {
                wFormatTag: 1, // PCM
                nChannels: 2,
                nSamplesPerSec: TAUX,
                nAvgBytesPerSec: TAUX * 4,
                nBlockAlign: 4,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                2_000_000, // 200 ms de tampon côté Windows
                0,
                &format,
                None,
            )?;
            let evenement: HANDLE = CreateEventW(None, false, false, None)?;
            let resultat = (|| -> Result<(), Erreur> {
                client.SetEventHandle(evenement)?;
                let lecture: IAudioCaptureClient = client.GetService()?;
                if let Ok(mut son) = SON.lock() {
                    *son = Some(TamponSon::new(TAUX, garder_ms));
                }
                client.Start()?;
                println!("Son : écoute du son de SF6 seulement, pour les vidéos.");
                let mut trames: Vec<[i16; 2]> = Vec::new();
                let mut verifie = Instant::now();
                loop {
                    let _ = WaitForSingleObject(evenement, 20);
                    while lecture.GetNextPacketSize()? > 0 {
                        let mut donnees = std::ptr::null_mut();
                        let mut nombre = 0u32;
                        let mut drapeaux = 0u32;
                        let mut heure = 0u64;
                        lecture.GetBuffer(
                            &mut donnees,
                            &mut nombre,
                            &mut drapeaux,
                            None,
                            Some(&mut heure),
                        )?;
                        let n = nombre as usize;
                        trames.clear();
                        if drapeaux & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0
                            || donnees.is_null()
                        {
                            trames.resize(n, [0, 0]);
                        } else {
                            let e = std::slice::from_raw_parts(donnees as *const i16, n * 2);
                            trames.extend(e.chunks_exact(2).map(|t| [t[0], t[1]]));
                        }
                        lecture.ReleaseBuffer(nombre)?;
                        // Heure de la 1re trame ; si Windows ne la donne pas, on la déduit.
                        let heure = if heure > 0 {
                            heure as i64
                        } else {
                            crate::horloge::maintenant() - (n as i64 * 10_000_000 / i64::from(TAUX))
                        };
                        if let Ok(mut son) = SON.lock()
                            && let Some(son) = son.as_mut()
                        {
                            son.ajouter(heure, &trames);
                        }
                    }
                    // SF6 silencieux : Windows n'envoie rien, on comble avec du silence
                    // (sauf les 50 dernières ms, qui peuvent encore arriver).
                    let maintenant = crate::horloge::maintenant();
                    if let Ok(mut son) = SON.lock()
                        && let Some(son) = son.as_mut()
                    {
                        son.silence_jusqua(maintenant - 500_000);
                    }
                    // SF6 fermé (ou relancé) : on arrête, la boucle le cherchera à nouveau.
                    if verifie.elapsed() > Duration::from_secs(3) {
                        verifie = Instant::now();
                        if trouver_processus(processus) != Some(pid) {
                            println!("Son : SF6 fermé.");
                            return Ok(());
                        }
                    }
                }
            })();
            let _ = client.Stop();
            let _ = CloseHandle(evenement);
            resultat
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 ms à 1000 Hz = une trame : pratique pour compter.
    const MS: i64 = 10_000;

    #[test]
    fn le_son_est_cale_sur_l_heure() {
        let mut son = TamponSon::new(1000, 10_000);
        son.ajouter(100 * MS, &[[1, 1]; 10]);
        son.ajouter(110 * MS, &[[2, 2]; 10]);
        assert_eq!(son.numero(100 * MS), Some(0));
        assert_eq!(son.numero(115 * MS), Some(15));
        // De la 5e trame jusqu'à 112 ms : 7 trames, 4 octets chacune.
        let (octets, suivant) = son.lire(5, 112 * MS);
        assert_eq!((octets.len(), suivant), (28, 12));
        assert_eq!(&octets[..4], &[1, 0, 1, 0]);
        assert_eq!(&octets[24..], &[2, 0, 2, 0]);
        // Pas plus loin que ce qui est reçu.
        assert_eq!(son.lire(12, 500 * MS).1, 20);
    }

    #[test]
    fn un_trou_devient_du_silence() {
        let mut son = TamponSon::new(1000, 10_000);
        son.ajouter(0, &[[5, 5]; 10]);
        // Rien pendant 30 ms, puis du son.
        son.ajouter(40 * MS, &[[7, 7]; 10]);
        assert_eq!(son.fin(), 50);
        let (octets, _) = son.lire(10, 40 * MS);
        assert!(octets.iter().all(|&o| o == 0));
        assert_eq!(son.lire(40, 41 * MS).0, vec![7, 0, 7, 0]);
    }

    #[test]
    fn le_silence_comble_puis_le_son_reprend_sans_doublon() {
        let mut son = TamponSon::new(1000, 10_000);
        son.ajouter(0, &[[1, 1]; 10]);
        son.silence_jusqua(50 * MS);
        assert_eq!(son.fin(), 50);
        // Du son arrive pour 30..60 ms : 30..50 déjà comblé, on garde 50..60.
        son.ajouter(30 * MS, &[[3, 3]; 30]);
        assert_eq!(son.fin(), 60);
    }

    #[test]
    fn les_vieilles_trames_sont_jetees() {
        let mut son = TamponSon::new(1000, 100);
        for i in 0..50 {
            son.ajouter(i * 10 * MS, &[[i as i16, 0]; 10]);
        }
        assert_eq!(son.trames.len(), 100);
        assert_eq!(son.retirees, 400);
        // Ce qui est jeté revient en silence.
        let (octets, suivant) = son.lire(398, 402 * MS);
        assert_eq!(suivant, 402);
        assert_eq!(&octets[..8], &[0; 8]);
        assert_eq!(&octets[8..10], &40i16.to_le_bytes());
    }
}
