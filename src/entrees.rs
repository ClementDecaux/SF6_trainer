//! La boucle qui lit la manette, reconnaît les coups et fait avancer le trial.
//!
//! Elle tourne dans son propre thread, pour ne jamais être ralentie par l'affichage.
//! Elle écrit ce qu'elle voit dans le terminal et dans le `Partage` lu par l'overlay.

use crate::affichage::{self, Etat};
use crate::capture::Capture;
use crate::combo::{Resultat, Trial};
use crate::controles::{Action, Controles};
use crate::motion::Historique;
use crate::notation::Directions;
use crate::partage::{Ligne, Partage};
use crate::timings::{ESSAIS_MIN, Timings};
use colored::Colorize;
use eframe::egui;
use gilrs::{Axis, Button, EventType, Gilrs};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Durée d'une frame de SF6 (60 images par seconde), en millisecondes.
const FRAME_MS: f64 = 1000.0 / 60.0;

/// Deux boutons appuyés à moins de 3 frames d'écart comptent comme simultanés (HP+HK).
const SIMULTANE_FRAMES: u64 = 3;

pub fn boucle(
    controles: Controles,
    mut trial: Trial,
    mut timings: Timings,
    mut capture: Capture,
    partage: Arc<Mutex<Partage>>,
    ctx: egui::Context,
) {
    let mut gilrs = match Gilrs::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("Impossible d'initialiser la lecture des manettes : {e}");
            return;
        }
    };

    println!("Manettes détectées :");
    for (id, gamepad) in gilrs.gamepads() {
        println!("  [{id}] {}", gamepad.name());
    }
    println!("Ctrl+C pour quitter. Joueur à gauche de l'écran (b = 4 = vers la gauche).\n");
    println!("{}  {}", trial.nom.bold(), affichage::trial(&trial));
    match &trial.fenetres {
        Some(_) => println!("Timing : fenêtres apprises dans timings.toml."),
        None => println!(
            "Timing : pas encore appris. Appuie sur Select après un vrai combo ({ESSAIS_MIN} fois)."
        ),
    }
    println!();

    let start = Instant::now();
    let mut dirs = Directions::default();
    let mut historique = Historique::default();
    let mut tenus: BTreeSet<Action> = BTreeSet::new();
    // Frame du dernier appui de chaque bouton (pour les appuis simultanés).
    let mut derniers_appuis: HashMap<Action, u64> = HashMap::new();

    // L'état en cours, depuis quelle frame, et ce qui a été appuyé à son début.
    let mut etat = Etat {
        direction: 5,
        boutons: vec![],
    };
    let mut debut = 0;
    let mut nouveaux: Vec<Action> = Vec::new();
    let mut coups: Vec<String> = Vec::new();
    // Dernier essai complet, en attente d'un Select (« c'était un vrai combo ») : frame, écarts.
    let mut a_confirmer: Option<(u64, Vec<u64>)> = None;
    let mut select_appuye = false;

    loop {
        // 1. On lit TOUS les événements en attente avant de calculer quoi que ce soit :
        //    une manette envoie souvent plusieurs changements en même temps.
        let mut appuyes: Vec<Action> = Vec::new();
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::ButtonPressed(Button::Select, _) => select_appuye = true,
                EventType::ButtonPressed(button, _) => {
                    if !changer_direction(&mut dirs, button, true) {
                        match controles.get(&format!("{button:?}")) {
                            Some(&action) => {
                                tenus.insert(action);
                                appuyes.push(action);
                            }
                            None => println!("     bouton {button:?} (pas dans controles.toml)"),
                        }
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if !changer_direction(&mut dirs, button, false)
                        && let Some(action) = controles.get(&format!("{button:?}"))
                    {
                        tenus.remove(action);
                    }
                }
                EventType::AxisChanged(axis, value, _) => match axis {
                    // Stick analogique : transformé en directions avec une zone morte.
                    Axis::LeftStickX => {
                        dirs.left = value < -0.5;
                        dirs.right = value > 0.5;
                    }
                    Axis::LeftStickY => {
                        dirs.down = value < -0.5;
                        dirs.up = value > 0.5;
                    }
                    _ => {}
                },
                EventType::Connected => {
                    println!("Manette connectée : {}", gilrs.gamepad(event.id).name())
                }
                EventType::Disconnected => println!("Manette déconnectée"),
                _ => {}
            }
        }

        let frame = (start.elapsed().as_secs_f64() * 1000.0 / FRAME_MS) as u64;
        let mut change = false;
        if select_appuye {
            select_appuye = false;
            confirmer(&mut trial, &mut timings, &mut a_confirmer, frame, &partage);
            change = true;
        }
        let direction = dirs.numpad();
        if direction != etat.direction {
            historique.ajouter(frame, direction);
            // Drive Rush par dash (66) : seulement si c'est ce que le trial attend.
            if direction == 6
                && trial.attendu() == "DR"
                && historique.dash(frame)
                && resultat_trial(
                    &mut trial,
                    &mut capture,
                    &["DR".to_string()],
                    "66 (DR)",
                    frame,
                    &partage,
                )
            {
                a_confirmer = Some((frame, trial.ecarts.clone()));
            }
        }

        // 2. Nouvel état (direction ou boutons différents, ou nouvel appui) :
        //    on affiche l'ancien avec sa durée, puis on démarre le nouveau.
        let nouvel_etat = Etat {
            direction,
            boutons: tenus.iter().copied().collect(),
        };
        if nouvel_etat != etat || !appuyes.is_empty() {
            if frame > debut || !nouveaux.is_empty() {
                println!(
                    "{}",
                    affichage::ligne(&etat, frame - debut, &nouveaux, &coups)
                );
                if let Ok(mut p) = partage.lock() {
                    p.ajouter_ligne(Ligne {
                        frames: frame - debut,
                        direction: etat.direction,
                        boutons: etat
                            .boutons
                            .iter()
                            .map(|b| (*b, nouveaux.contains(b)))
                            .collect(),
                        coups: coups.clone(),
                    });
                }
            }

            // 3. Chaque bouton appuyé : toutes les lectures possibles, puis le trial.
            coups.clear();
            for &action in &appuyes {
                let candidats = lectures(
                    action,
                    direction,
                    frame,
                    &historique,
                    &tenus,
                    &derniers_appuis,
                );
                derniers_appuis.insert(action, frame);
                // On affiche la lecture attendue par le trial si elle y est, sinon la première.
                let coup = candidats
                    .iter()
                    .find(|c| c.as_str() == trial.attendu())
                    .unwrap_or(&candidats[0])
                    .clone();
                if resultat_trial(&mut trial, &mut capture, &candidats, &coup, frame, &partage) {
                    a_confirmer = Some((frame, trial.ecarts.clone()));
                }
                coups.push(coup);
            }

            etat = nouvel_etat;
            debut = frame;
            nouveaux = appuyes;
            change = true;
        }

        if trial.temps_depasse(frame) {
            println!("{}", "      ⏱ trop lent, on recommence".red());
            println!("{}", affichage::trial(&trial));
            capture.annuler();
            if let Ok(mut p) = partage.lock() {
                p.index = 0;
                p.message("Trop lent, on recommence".to_string(), false);
            }
            change = true;
        }

        // Prévenir l'overlay qu'il y a du nouveau à dessiner.
        if change {
            ctx.request_repaint();
        }

        // On regarde la manette toutes les millisecondes (largement plus vite qu'une frame).
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Toutes les façons de lire un appui : « 4MK », « 236MK », « DI »...
/// La première est la lecture principale (mouvement spécial prioritaire, sinon coup normal).
fn lectures(
    action: Action,
    direction: u8,
    frame: u64,
    historique: &Historique,
    tenus: &BTreeSet<Action>,
    derniers_appuis: &HashMap<Action, u64>,
) -> Vec<String> {
    let mut candidats = Vec::new();
    match action {
        Action::DP | Action::DI | Action::Throw => candidats.push(action.to_string()),
        _ => {
            for m in historique.reconnaitre_tous(frame) {
                candidats.push(format!("{}{action}", m.nom()));
            }
            candidats.push(format!("{direction}{action}"));
        }
    }

    // Deux boutons presque en même temps : MP+MK = Drive Parry, HP+HK = Drive Impact.
    let (autre, nom) = match action {
        Action::MP => (Action::MK, "DP"),
        Action::MK => (Action::MP, "DP"),
        Action::HP => (Action::HK, "DI"),
        Action::HK => (Action::HP, "DI"),
        _ => return candidats,
    };
    if tenus.contains(&autre)
        && derniers_appuis
            .get(&autre)
            .is_some_and(|&f| frame.saturating_sub(f) < SIMULTANE_FRAMES)
    {
        candidats.push(nom.to_string());
    }
    candidats
}

/// Donne un coup au trial, l'affiche dans le terminal, met l'overlay à jour
/// et dit à la capture vidéo quand filmer. Renvoie `true` si toutes les étapes sont faites.
fn resultat_trial(
    trial: &mut Trial,
    capture: &mut Capture,
    candidats: &[String],
    coup: &str,
    frame: u64,
    partage: &Mutex<Partage>,
) -> bool {
    let resultat = trial.coup(candidats, frame);
    suivre_video(trial, capture, &resultat, frame);
    let Ok(mut p) = partage.lock() else {
        return false;
    };
    p.index = trial.index;
    p.ecarts = trial.ecarts.clone();
    if matches!(resultat, Resultat::Avance | Resultat::Reussi)
        && let Some(&ecart) = trial.ecarts.last()
    {
        println!("{}", format!("      +{ecart} f").dimmed());
    }
    match resultat {
        Resultat::Avance => println!("{}", affichage::trial(trial)),
        Resultat::Reussi if trial.erreurs_timing.is_empty() => {
            println!("{}", "      ★ COMBO RÉUSSI ★".bright_green().bold());
            println!("{}", affichage::trial(trial));
            crate::son::reussi();
            p.reussites += 1;
            p.message("COMBO RÉUSSI !".to_string(), true);
            return true;
        }
        Resultat::Reussi => {
            let erreurs = trial.erreurs_timing.join(", ");
            println!("{}", format!("      ✗ inputs bons, mais {erreurs}").red());
            println!("{}", affichage::trial(trial));
            p.message(format!("Timing : {erreurs}"), false);
            return true;
        }
        Resultat::Rate { attendu } => {
            println!(
                "{}",
                format!("      ✗ attendu {attendu}, reçu {coup}").red()
            );
            println!("{}", affichage::trial(trial));
            p.message(format!("Raté : attendu {attendu}, reçu {coup}"), false);
        }
        Resultat::Ignore => {}
    }
    false
}

/// Select après un combo : « c'était un vrai combo ». Ses écarts sont ajoutés aux
/// fenêtres de timing (`timings.toml`).
fn confirmer(
    trial: &mut Trial,
    timings: &mut Timings,
    a_confirmer: &mut Option<(u64, Vec<u64>)>,
    frame: u64,
    partage: &Mutex<Partage>,
) {
    // 10 secondes pour confirmer, après, on ne sait plus de quel essai on parle.
    const DELAI_CONFIRMATION: u64 = 600;
    let texte = match a_confirmer.take() {
        Some((fin, ecarts)) if frame.saturating_sub(fin) <= DELAI_CONFIRMATION => {
            let n = timings.apprendre(&trial.nom, &ecarts);
            if let Err(e) = timings.enregistrer() {
                eprintln!("{e}");
            }
            trial.fenetres = timings.fenetres(&trial.nom, trial.etapes.len());
            if let Ok(mut p) = partage.lock() {
                p.fenetres = trial.fenetres.clone();
            }
            if n < ESSAIS_MIN {
                format!("Timing : {n}/{ESSAIS_MIN} vrais combos confirmés")
            } else {
                format!("Timing appris ({n} vrais combos)")
            }
        }
        _ => "Select : rien à confirmer, finis d'abord le combo".to_string(),
    };
    println!("{}", format!("      ◆ {texte}").cyan());
    if let Ok(mut p) = partage.lock() {
        p.message(texte, true);
    }
}

/// La vidéo suit le trial : on filme dès la 1re étape, on garde si c'est réussi.
fn suivre_video(trial: &Trial, capture: &mut Capture, resultat: &Resultat, frame: u64) {
    match resultat {
        Resultat::Avance => {
            if trial.index == 1 {
                capture.debut_essai(frame);
            }
            capture.etape(&trial.etapes[trial.index - 1].notation, frame);
        }
        Resultat::Reussi => {
            if let Some(derniere) = trial.etapes.last() {
                capture.etape(&derniere.notation, frame);
            }
            // Hors timing : le combo n'est sûrement pas passé, on ne garde pas la vidéo.
            if trial.erreurs_timing.is_empty() {
                capture.reussi();
            } else {
                capture.annuler();
            }
        }
        Resultat::Rate { .. } => {
            // Le coup raté peut être le début d'un nouvel essai (index revenu à 1).
            if trial.index == 1 {
                capture.debut_essai(frame);
                capture.etape(&trial.etapes[0].notation, frame);
            } else {
                capture.annuler();
            }
        }
        Resultat::Ignore => {}
    }
}

/// Met à jour les directions si le bouton est une flèche de la croix (D-pad).
/// Les leverless en mode XInput envoient leurs directions comme une croix.
/// Renvoie `true` si le bouton était une direction.
fn changer_direction(dirs: &mut Directions, button: Button, pressed: bool) -> bool {
    match button {
        Button::DPadUp => dirs.up = pressed,
        Button::DPadDown => dirs.down = pressed,
        Button::DPadLeft => dirs.left = pressed,
        Button::DPadRight => dirs.right = pressed,
        _ => return false,
    }
    true
}
