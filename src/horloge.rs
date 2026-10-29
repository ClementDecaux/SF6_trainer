//! L'horloge des frames, calée sur les images de SF6.
//!
//! Le jeu tourne à 60 images par seconde, mais ses frames ne commencent pas au même
//! moment que les nôtres : si on compte les frames depuis le lancement du programme,
//! un appui fait près d'une limite tombe une frame trop tôt ou trop tard par rapport au jeu.
//!
//! La capture vidéo reçoit chaque image du jeu avec son heure exacte. On en déduit la
//! « phase » du jeu (où commencent ses frames), et on découpe le temps de la même façon.
//! Sans capture (SF6 pas trouvé ou vidéo coupée), on compte depuis le lancement, comme avant.

// Hors Windows, pas de capture ni de XInput : une partie du code n'y sert pas.
#![cfg_attr(not(windows), allow(dead_code))]

use std::sync::atomic::{AtomicI64, Ordering};

/// Une frame à 60 images par seconde, en unités de 100 ns (l'unité de l'horloge Windows).
const FRAME: f64 = 10_000_000.0 / 60.0;

/// Début d'une frame du jeu, ramené dans [0, FRAME), en unités de 100 ns. -1 = inconnu.
static PHASE: AtomicI64 = AtomicI64::new(-1);

/// Réglage fin (`decalage_ms` dans capture.toml), en unités de 100 ns : le jeu lit la
/// manette un peu avant d'afficher l'image, on décale nos limites de frames d'autant.
static DECALAGE: AtomicI64 = AtomicI64::new(0);

pub fn regler_decalage(ms: f64) {
    DECALAGE.store((ms * 10_000.0) as i64, Ordering::Relaxed);
}

/// Vrai dès que les images du jeu arrivent (frames calées sur SF6).
pub fn synchro_active() -> bool {
    PHASE.load(Ordering::Relaxed) >= 0
}

/// Où tombe un instant dans sa frame, en millisecondes (0 = tout début, 16,6 = toute fin).
/// Affiché dans le terminal pour trouver le bon `decalage_ms`.
pub fn position_ms(temps: i64) -> f64 {
    (temps as f64 - debut_des_frames()).rem_euclid(FRAME) / 10_000.0
}

/// Le début des frames (phase du jeu + réglage fin), en unités de 100 ns.
fn debut_des_frames() -> f64 {
    let phase = PHASE.load(Ordering::Relaxed).max(0);
    (phase + DECALAGE.load(Ordering::Relaxed)) as f64
}

/// Nouvelle image du jeu, montrée à l'écran à `temps` (unités de 100 ns, horloge Windows).
/// Appelé par le thread de capture à chaque image.
pub fn image_du_jeu(temps: i64) {
    let mesure = temps.rem_euclid(FRAME as i64) as f64;
    let ancienne = PHASE.load(Ordering::Relaxed);
    let phase = if ancienne < 0 {
        mesure
    } else {
        lisser(ancienne as f64, mesure)
    };
    PHASE.store(phase as i64, Ordering::Relaxed);
}

/// Moyenne glissante d'une phase : les heures d'affichage tremblent un peu (écran à
/// 144 Hz, par exemple), on suit donc la tendance plutôt que chaque image.
/// Le calcul tient compte du « tour » : 16,6 ms et 0,1 ms sont deux phases très proches.
fn lisser(ancienne: f64, mesure: f64) -> f64 {
    let mut ecart = mesure - ancienne;
    if ecart > FRAME / 2.0 {
        ecart -= FRAME;
    } else if ecart < -FRAME / 2.0 {
        ecart += FRAME;
    }
    (ancienne + ecart * 0.05).rem_euclid(FRAME)
}

/// Le numéro de frame d'un instant (unités de 100 ns), calé sur le jeu si possible.
pub fn frame(temps: i64) -> u64 {
    ((temps as f64 - debut_des_frames()) / FRAME)
        .floor()
        .max(0.0) as u64
}

/// L'heure actuelle, sur la même horloge que les images de la capture.
#[cfg(windows)]
pub fn maintenant() -> i64 {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::Performance::{
        QueryPerformanceCounter, QueryPerformanceFrequency,
    };
    static FREQUENCE: OnceLock<i64> = OnceLock::new();
    let frequence = *FREQUENCE.get_or_init(|| {
        let mut f = 0;
        unsafe { QueryPerformanceFrequency(&mut f) };
        f.max(1)
    });
    let mut compteur = 0;
    unsafe { QueryPerformanceCounter(&mut compteur) };
    (compteur as i128 * 10_000_000 / frequence as i128) as i64
}

#[cfg(not(windows))]
pub fn maintenant() -> i64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static DEBUT: OnceLock<Instant> = OnceLock::new();
    DEBUT.get_or_init(Instant::now).elapsed().as_nanos() as i64 / 100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_lissage_passe_par_le_bon_cote() {
        // Phase 166 000 (fin de frame), mesure 600 (tout début de la suivante) :
        // c'est un petit écart de ~1 200, pas un grand saut en arrière.
        let p = lisser(166_000.0, 600.0);
        assert!(!(1_000.0..=166_000.0).contains(&p), "phase {p}");
    }

    #[test]
    fn les_frames_suivent_la_phase() {
        // Sans phase, la frame 0 va de 0 à 166 666.
        assert_eq!(frame(100_000), 0);
        assert_eq!(frame(200_000), 1);
    }
}
