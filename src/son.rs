//! Un petit « ding » quand un combo est réussi.
//!
//! Le son est fabriqué par le programme (deux notes, comme un carillon) : pas de fichier
//! audio à fournir. Il est joué par Windows (`PlaySoundW`), sans bloquer la lecture de la manette.
//! `son = false` dans `overlay.toml` pour le couper.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Fréquence d'échantillonnage : 44 100 valeurs par seconde, comme un CD.
const ECHANTILLONS_PAR_SECONDE: u32 = 44_100;
/// Volume, de 0 à 1.
const VOLUME: f32 = 0.35;
/// Le carillon : do puis sol aigus (fréquence en Hz, durée en secondes).
const NOTES: [(f32, f32); 2] = [(1046.5, 0.09), (1568.0, 0.22)];

static ACTIF: AtomicBool = AtomicBool::new(true);

pub fn activer(oui: bool) {
    ACTIF.store(oui, Ordering::Relaxed);
}

/// Joue le son du combo réussi (Windows seulement).
pub fn reussi() {
    if !ACTIF.load(Ordering::Relaxed) {
        return;
    }
    // Le son doit rester en mémoire pendant qu'il est joué : on le garde pour toujours.
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    let wav = WAV.get_or_init(|| fabriquer_wav(&NOTES));
    jouer(wav);
}

#[cfg(windows)]
fn jouer(wav: &'static [u8]) {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    // SND_MEMORY : le « nom » du son est en fait l'adresse du fichier WAV en mémoire.
    // SND_ASYNC : on n'attend pas la fin du son.
    unsafe {
        PlaySoundW(
            wav.as_ptr().cast(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

#[cfg(not(windows))]
fn jouer(_wav: &'static [u8]) {}

/// Fabrique un fichier WAV complet (en-tête + son), mono, 16 bits.
fn fabriquer_wav(notes: &[(f32, f32)]) -> Vec<u8> {
    let mut valeurs: Vec<i16> = Vec::new();
    for &(frequence, duree) in notes {
        let n = (duree * ECHANTILLONS_PAR_SECONDE as f32) as usize;
        for i in 0..n {
            let t = i as f32 / ECHANTILLONS_PAR_SECONDE as f32;
            // Attaque très courte (pas de « clic »), puis le son s'éteint doucement.
            let attaque = (t / 0.004).min(1.0);
            let extinction = (-t * 14.0).exp();
            let onde = (2.0 * std::f32::consts::PI * frequence * t).sin();
            valeurs.push((onde * attaque * extinction * VOLUME * i16::MAX as f32) as i16);
        }
    }

    let taille_donnees = (valeurs.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + taille_donnees as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + taille_donnees).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // taille du bloc « fmt »
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&ECHANTILLONS_PAR_SECONDE.to_le_bytes());
    wav.extend_from_slice(&(ECHANTILLONS_PAR_SECONDE * 2).to_le_bytes()); // octets par seconde
    wav.extend_from_slice(&2u16.to_le_bytes()); // octets par échantillon
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits par échantillon
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&taille_donnees.to_le_bytes());
    for v in valeurs {
        wav.extend_from_slice(&v.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_wav_a_un_en_tete_valide() {
        let wav = fabriquer_wav(&NOTES);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let taille = u32::from_le_bytes(wav[4..8].try_into().unwrap());
        assert_eq!(taille as usize, wav.len() - 8);
        let donnees = u32::from_le_bytes(wav[40..44].try_into().unwrap());
        assert_eq!(donnees as usize, wav.len() - 44);
    }
}
