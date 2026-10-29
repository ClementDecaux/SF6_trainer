//! Étapes 5 et 6 : le trial en overlay par-dessus SF6.
//!
//! Lance avec `cargo run` (ou `cargo run -- 2` pour le 2e combo de `combos.toml`).
//! Deux threads :
//! - celui de la manette (`entrees.rs`) lit les inputs et fait avancer le trial ;
//! - le thread principal dessine l'overlay (`overlay.rs`).
//!
//! Ils partagent l'état du trial à travers un `Arc<Mutex<Partage>>` (`partage.rs`).
//! Un troisième thread (`capture.rs`, Windows) filme SF6 et garde les combos réussis.
//! Fichiers de config : `controles.toml`, `combos.toml`, `overlay.toml`, `capture.toml`.

mod affichage;
mod capture;
mod clavier;
mod combo;
mod controles;
mod entrees;
mod motion;
mod notation;
mod overlay;
mod partage;
mod son;
mod timings;

use combo::Trial;
use eframe::egui;
use partage::Partage;
use std::sync::{Arc, Mutex};

fn main() {
    // Active les couleurs dans les anciens terminaux Windows.
    #[cfg(windows)]
    let _ = colored::control::set_virtual_terminal(true);

    if let Err(e) = lancer() {
        eprintln!("{e}");
    }
}

fn lancer() -> Result<(), String> {
    let controles = controles::charger()?;
    let (mut trial, personnage, inputs) = choisir_trial()?;
    let timings = timings::charger()?;
    trial.fenetres = timings.fenetres(&trial.nom, trial.etapes.len());
    let config = overlay::charger()?;
    son::activer(config.son);
    let capture = capture::lancer(capture::charger()?, &personnage, &trial.nom, &inputs);

    let mut etat = Partage::new(&trial.nom, trial.etapes.clone());
    etat.fenetres = trial.fenetres.clone();
    let partage = Arc::new(Mutex::new(etat));

    // La fenêtre : sans bordure, au premier plan, les clics passent au travers,
    // et transparente si `transparence = true` dans overlay.toml.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SF6 Trainer")
            .with_transparent(config.transparence)
            .with_decorations(false)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_position([config.x, config.y])
            .with_inner_size([config.largeur, config.hauteur]),
        ..Default::default()
    };

    eframe::run_native(
        "SF6 Trainer",
        options,
        Box::new(move |cc| {
            // Le thread de la manette reçoit une copie du `Context` pour demander
            // à l'overlay de se redessiner quand quelque chose change.
            let ctx = cc.egui_ctx.clone();
            let partage_manette = Arc::clone(&partage);
            std::thread::spawn(move || {
                entrees::boucle(controles, trial, timings, capture, partage_manette, ctx)
            });
            Ok(Box::new(overlay::Overlay::new(partage, &config)))
        }),
    )
    .map_err(|e| format!("erreur de l'overlay : {e}"))
}

/// Charge `combos.toml` et prend le combo demandé (`cargo run -- 2`), le premier par défaut.
/// Renvoie aussi le personnage et le combo tel qu'il est écrit (pour le `.json` des vidéos).
fn choisir_trial() -> Result<(Trial, String, String), String> {
    let fichier = combo::charger()?;
    let numero: usize = match std::env::args().nth(1) {
        Some(arg) => arg
            .parse()
            .map_err(|_| format!("« {arg} » n'est pas un numéro de combo"))?,
        None => 1,
    };
    let choisi = fichier
        .combo
        .get(numero.wrapping_sub(1))
        .ok_or(format!("pas de combo n°{numero} dans combos.toml"))?;
    let etapes = combo::parser(&choisi.inputs)?;
    println!(
        "{} : {} combo(s) dans combos.toml, n°{numero} chargé.",
        fichier.personnage,
        fichier.combo.len()
    );
    Ok((
        Trial::new(&choisi.nom, etapes),
        fichier.personnage.clone(),
        choisi.inputs.clone(),
    ))
}
