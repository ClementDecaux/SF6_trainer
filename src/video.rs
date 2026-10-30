//! Lecture des vidéos des combos (MP4), image par image, pour l'interface.
//!
//! Sous Windows, le décodage est fait par Windows lui-même (Media Foundation) : rien à
//! installer. Toute la vidéo est décodée dans un thread à part, en taille réduite (960 de
//! large au plus), et gardée en mémoire : on peut ensuite aller d'une image à l'autre
//! dans les deux sens instantanément.

#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Largeur maximale gardée (une vidéo 1080p est réduite de moitié).
const LARGEUR_MAX: u32 = 960;
/// Au-delà, on arrête de décoder (15 s à 60 images par seconde).
const IMAGES_MAX: usize = 900;

/// Une image décodée : son heure depuis le début de la vidéo (100 ns) et ses pixels RGB.
pub struct Image {
    pub temps: i64,
    pub rgb: Vec<u8>,
}

/// Ce que le thread de décodage remplit au fur et à mesure.
#[derive(Default)]
pub struct Decodage {
    pub largeur: u32,
    pub hauteur: u32,
    pub images: Vec<Image>,
    pub fini: bool,
    pub erreur: Option<String>,
}

/// Une vidéo en cours de décodage (ou décodée).
pub struct Video {
    pub chemin: PathBuf,
    pub decodage: Arc<Mutex<Decodage>>,
}

impl Video {
    /// Lance le décodage de la vidéo dans un thread.
    pub fn ouvrir(chemin: &Path) -> Video {
        let decodage = Arc::new(Mutex::new(Decodage::default()));
        let partage = Arc::clone(&decodage);
        let fichier = chemin.to_path_buf();
        std::thread::spawn(move || {
            if let Err(e) = decoder(&fichier, &partage)
                && let Ok(mut d) = partage.lock()
            {
                d.erreur = Some(e);
            }
            if let Ok(mut d) = partage.lock() {
                d.fini = true;
            }
        });
        Video {
            chemin: chemin.to_path_buf(),
            decodage,
        }
    }
}

/// Réduit une image BGRA (`pas` octets par ligne) d'un facteur entier, en RGB.
/// Chaque pixel de sortie est la moyenne d'un carré de `facteur` x `facteur` pixels.
pub fn reduire(bgra: &[u8], largeur: u32, hauteur: u32, pas: usize, facteur: u32) -> Vec<u8> {
    let (l, h) = (largeur / facteur, hauteur / facteur);
    let f = facteur as usize;
    let mut rgb = Vec::with_capacity((l * h * 3) as usize);
    for y in 0..h as usize {
        for x in 0..l as usize {
            let mut somme = [0u32; 3];
            for dy in 0..f {
                let ligne = (y * f + dy) * pas;
                for dx in 0..f {
                    let i = ligne + (x * f + dx) * 4;
                    if let Some(p) = bgra.get(i..i + 3) {
                        somme[0] += u32::from(p[2]);
                        somme[1] += u32::from(p[1]);
                        somme[2] += u32::from(p[0]);
                    }
                }
            }
            let n = (f * f) as u32;
            rgb.extend(somme.iter().map(|s| (s / n) as u8));
        }
    }
    rgb
}

/// Le facteur de réduction pour ne pas dépasser `LARGEUR_MAX`.
pub fn facteur(largeur: u32) -> u32 {
    largeur.div_ceil(LARGEUR_MAX).max(1)
}

#[cfg(not(windows))]
fn decoder(_chemin: &Path, _decodage: &Mutex<Decodage>) -> Result<(), String> {
    Err("lecture des vidéos disponible seulement sous Windows".to_string())
}

#[cfg(windows)]
fn decoder(chemin: &Path, decodage: &Mutex<Decodage>) -> Result<(), String> {
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::HSTRING;

    let erreur = |quoi: &str, e: windows::core::Error| format!("{quoi} : {e}");
    let flux = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    unsafe {
        // Déjà initialisé dans ce thread : sans importance.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| erreur("Media Foundation", e))?;

        let mut attributs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attributs, 1).map_err(|e| erreur("attributs", e))?;
        let attributs = attributs.ok_or("attributs vides")?;
        // Conversion des couleurs (H.264 -> RGB32) faite par Windows.
        attributs
            .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
            .map_err(|e| erreur("attributs", e))?;
        let url = HSTRING::from(chemin.as_os_str());
        let lecteur = MFCreateSourceReaderFromURL(&url, &attributs)
            .map_err(|e| erreur("ouverture de la vidéo", e))?;

        let type_voulu = MFCreateMediaType().map_err(|e| erreur("type", e))?;
        type_voulu
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| erreur("type", e))?;
        type_voulu
            .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
            .map_err(|e| erreur("type", e))?;
        lecteur
            .SetCurrentMediaType(flux, None, &type_voulu)
            .map_err(|e| erreur("conversion en RGB", e))?;
        let type_reel = lecteur
            .GetCurrentMediaType(flux)
            .map_err(|e| erreur("type", e))?;
        let taille = type_reel
            .GetUINT64(&MF_MT_FRAME_SIZE)
            .map_err(|e| erreur("taille", e))?;
        let (largeur, hauteur) = ((taille >> 32) as u32, taille as u32);
        // Pas négatif : l'image est rangée de bas en haut.
        let pas = type_reel
            .GetUINT32(&MF_MT_DEFAULT_STRIDE)
            .map(|p| p as i32)
            .unwrap_or(largeur as i32 * 4);
        let a_l_envers = pas < 0;
        let pas = pas.unsigned_abs() as usize;
        let f = facteur(largeur);
        if let Ok(mut d) = decodage.lock() {
            d.largeur = largeur / f;
            d.hauteur = hauteur / f;
        }

        let mut premier: Option<i64> = None;
        let mut retournee = Vec::new();
        loop {
            let mut drapeaux = 0u32;
            let mut temps = 0i64;
            let mut echantillon: Option<IMFSample> = None;
            lecteur
                .ReadSample(
                    flux,
                    0,
                    None,
                    Some(&mut drapeaux),
                    Some(&mut temps),
                    Some(&mut echantillon),
                )
                .map_err(|e| erreur("décodage", e))?;
            if drapeaux & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(echantillon) = echantillon else {
                continue;
            };
            let tampon = echantillon
                .ConvertToContiguousBuffer()
                .map_err(|e| erreur("image", e))?;
            let mut pointeur = std::ptr::null_mut();
            let mut longueur = 0u32;
            tampon
                .Lock(&mut pointeur, None, Some(&mut longueur))
                .map_err(|e| erreur("image", e))?;
            let octets = std::slice::from_raw_parts(pointeur, longueur as usize);
            let source: &[u8] = if a_l_envers {
                retournee.clear();
                for y in (0..hauteur as usize).rev() {
                    if let Some(ligne) = octets.get(y * pas..(y + 1) * pas) {
                        retournee.extend_from_slice(ligne);
                    }
                }
                &retournee
            } else {
                octets
            };
            let rgb = reduire(source, largeur, hauteur, pas, f);
            let _ = tampon.Unlock();
            let debut = *premier.get_or_insert(temps);
            let Ok(mut d) = decodage.lock() else {
                break;
            };
            d.images.push(Image {
                temps: temps - debut,
                rgb,
            });
            if d.images.len() >= IMAGES_MAX {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduire_de_moitie_fait_la_moyenne() {
        // 2 x 2 pixels BGRA : bleu pur, rouge pur, noir, blanc.
        let bgra = [
            255, 0, 0, 255, 0, 0, 255, 255, //
            0, 0, 0, 255, 255, 255, 255, 255,
        ];
        let rgb = reduire(&bgra, 2, 2, 8, 2);
        assert_eq!(rgb, vec![127, 63, 127]);
        assert_eq!(facteur(1920), 2);
        assert_eq!(facteur(960), 1);
        assert_eq!(facteur(2560), 3);
    }
}
