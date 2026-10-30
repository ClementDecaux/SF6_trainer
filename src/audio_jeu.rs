//! Le son du jeu dans les vidéos.
//!
//! Windows sait « écouter » ce qui sort des haut-parleurs (WASAPI en mode loopback). Un thread
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

/// Commence à écouter le son de Windows (sans effet hors Windows).
pub fn demarrer(avant_ms: u64) {
    #[cfg(windows)]
    std::thread::spawn(move || wasapi::boucle(avant_ms + MARGE_MS));
    #[cfg(not(windows))]
    let _ = avant_ms;
}

/// Convertit un échantillon flottant (-1 à 1) en 16 bits.
fn vers_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0) as i16
}

#[cfg(windows)]
mod wasapi {
    use super::{SON, TamponSon, vers_i16};
    use std::time::Duration;
    use windows::Win32::Media::Audio::{
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
        IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
        WAVEFORMATEXTENSIBLE, eConsole, eRender,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    };

    type Erreur = Box<dyn std::error::Error>;

    /// Écoute le son ; en cas d'erreur (casque débranché…), recommence un peu plus tard.
    pub fn boucle(garder_ms: u64) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut prevenu = false;
        loop {
            if let Err(e) = ecouter(garder_ms)
                && !prevenu
            {
                eprintln!("Son : impossible d'écouter le son de Windows ({e}). Vidéos sans son.");
                prevenu = true;
            }
            if let Ok(mut son) = SON.lock() {
                *son = None;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    }

    /// Comment lire les échantillons donnés par Windows.
    #[derive(Clone, Copy)]
    enum Format {
        Flottant,
        Entier16,
    }

    fn ecouter(garder_ms: u64) -> Result<(), Erreur> {
        unsafe {
            let appareils: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let sortie = appareils.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let client: IAudioClient = sortie.Activate(CLSCTX_ALL, None)?;
            let melange = client.GetMixFormat()?;
            let resultat = (|| -> Result<(), Erreur> {
                // Structures « packed » : on copie chaque champ avant de s'en servir.
                let fmt: WAVEFORMATEX = std::ptr::read_unaligned(melange);
                let (canaux, bits, taux, tag) = (
                    fmt.nChannels,
                    fmt.wBitsPerSample,
                    fmt.nSamplesPerSec,
                    fmt.wFormatTag,
                );
                let canaux = usize::from(canaux.max(1));
                let tag = if tag == 0xFFFE {
                    // WAVE_FORMAT_EXTENSIBLE : le vrai format est dans SubFormat.
                    let ext = std::ptr::read_unaligned(melange as *const WAVEFORMATEXTENSIBLE);
                    let sous_format = ext.SubFormat;
                    sous_format.data1 as u16
                } else {
                    tag
                };
                let format = match (tag, bits) {
                    (3, 32) => Format::Flottant,
                    (1, 16) => Format::Entier16,
                    _ => return Err(format!("format du son non géré ({tag}, {bits} bits)").into()),
                };
                // 200 ms de tampon côté Windows.
                client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK,
                    2_000_000,
                    0,
                    melange,
                    None,
                )?;
                let lecture: IAudioCaptureClient = client.GetService()?;
                if let Ok(mut son) = SON.lock() {
                    *son = Some(TamponSon::new(taux, garder_ms));
                }
                client.Start()?;
                println!("Son : écoute du son de Windows ({taux} Hz) pour les vidéos.");
                let mut trames: Vec<[i16; 2]> = Vec::new();
                loop {
                    std::thread::sleep(Duration::from_millis(10));
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
                        trames.clear();
                        let n = nombre as usize;
                        if drapeaux & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0
                            || donnees.is_null()
                        {
                            trames.resize(n, [0, 0]);
                        } else {
                            convertir(donnees, n, canaux, format, &mut trames);
                        }
                        lecture.ReleaseBuffer(nombre)?;
                        if let Ok(mut son) = SON.lock()
                            && let Some(son) = son.as_mut()
                        {
                            son.ajouter(heure as i64, &trames);
                        }
                    }
                    // Rien ne joue : Windows n'envoie rien, on comble avec du silence
                    // (sauf les 50 dernières ms, qui peuvent encore arriver).
                    let maintenant = crate::horloge::maintenant();
                    if let Ok(mut son) = SON.lock()
                        && let Some(son) = son.as_mut()
                    {
                        son.silence_jusqua(maintenant - 500_000);
                    }
                }
            })();
            CoTaskMemFree(Some(melange as *const _));
            resultat
        }
    }

    /// Les échantillons de Windows (N canaux) en trames stéréo 16 bits.
    unsafe fn convertir(
        donnees: *const u8,
        n: usize,
        canaux: usize,
        format: Format,
        trames: &mut Vec<[i16; 2]>,
    ) {
        let droite = if canaux > 1 { 1 } else { 0 };
        match format {
            Format::Flottant => {
                let e = unsafe { std::slice::from_raw_parts(donnees as *const f32, n * canaux) };
                trames.extend(
                    e.chunks_exact(canaux)
                        .map(|t| [vers_i16(t[0]), vers_i16(t[droite])]),
                );
            }
            Format::Entier16 => {
                let e = unsafe { std::slice::from_raw_parts(donnees as *const i16, n * canaux) };
                trames.extend(e.chunks_exact(canaux).map(|t| [t[0], t[droite]]));
            }
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

    #[test]
    fn conversion_des_flottants() {
        assert_eq!(vers_i16(0.0), 0);
        assert_eq!(vers_i16(1.0), 32767);
        assert_eq!(vers_i16(-2.0), -32767);
    }
}
