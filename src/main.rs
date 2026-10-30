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
mod ecran;
mod ecran_modeles;
mod entrees;
mod horloge;
mod manette;
mod motion;
mod notation;
mod overlay;
mod partage;
mod son;
mod timings;

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
    let (personnage, combos, choisi) = choisir_combo()?;
    let timings = timings::charger()?;
    let trial = combo::creer_trial(&combos[choisi], &timings)?;
    let inputs = combos[choisi].inputs.clone();
    let config = overlay::charger()?;
    son::activer(config.son);
    let capture = capture::lancer(capture::charger()?, &personnage, &trial.nom, &inputs);

    let mut etat = Partage::new(&trial.nom, trial.etapes.clone());
    etat.combos = combos.iter().map(|c| c.nom.clone()).collect();
    etat.combo_index = choisi;
    etat.enregistrement = capture.enregistre;
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
            // Pour afficher les icônes SVG des boutons.
            egui_extras::install_image_loaders(&ctx);
            let partage_manette = Arc::clone(&partage);
            std::thread::spawn(move || {
                entrees::boucle(
                    controles,
                    (trial, combos),
                    timings,
                    capture,
                    partage_manette,
                    ctx,
                )
            });
            Ok(Box::new(overlay::Overlay::new(partage, &config)))
        }),
    )
    .map_err(|e| format!("erreur de l'overlay : {e}"))
}

/// Charge `combos.toml` et prend le combo demandé (`cargo run -- 2`), le premier par défaut.
/// On peut ensuite en changer avec les flèches de l'overlay.
/// Renvoie le personnage, tous les combos et le numéro (à partir de 0) du combo choisi.
fn choisir_combo() -> Result<(String, Vec<combo::ComboFichier>, usize), String> {
    let fichier = combo::charger()?;
    let numero: usize = match std::env::args().nth(1) {
        Some(arg) => arg
            .parse()
            .map_err(|_| format!("« {arg} » n'est pas un numéro de combo"))?,
        None => 1,
    };
    if numero == 0 || numero > fichier.combo.len() {
        return Err(format!("pas de combo n°{numero} dans combos.toml"));
    }
    println!(
        "{} : {} combo(s) dans combos.toml.",
        fichier.personnage,
        fichier.combo.len()
    );
    for (i, c) in fichier.combo.iter().enumerate() {
        let marque = if i + 1 == numero { "►" } else { " " };
        println!("  {marque} {:>2}. {}", i + 1, c.nom);
    }
    Ok((fichier.personnage, fichier.combo, numero - 1))
}
