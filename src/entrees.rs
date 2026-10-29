//! La boucle qui lit la manette, reconnaît les coups et fait avancer le trial.
//!
//! Elle tourne dans son propre thread, pour ne jamais être ralentie par l'affichage.
//! Elle écrit ce qu'elle voit dans le terminal et dans le `Partage` lu par l'overlay.

use crate::affichage::{self, Etat};
use crate::combo::{Resultat, Trial};
use crate::controles::{Action, Controles};
use crate::motion::Historique;
use crate::notation::Directions;
use crate::partage::{Ligne, Partage};
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
    println!("{}  {}\n", trial.nom.bold(), affichage::trial(&trial));

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

    loop {
        // 1. On lit TOUS les événements en attente avant de calculer quoi que ce soit :
        //    une manette envoie souvent plusieurs changements en même temps.
        let mut appuyes: Vec<Action> = Vec::new();
        while let Some(event) = gilrs.next_event() {
            match event.event {
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
        let direction = dirs.numpad();
        let mut change = false;
        if direction != etat.direction {
            historique.ajouter(frame, direction);
            // Drive Rush par dash (66) : seulement si c'est ce que le trial attend.
            if direction == 6 && trial.attendu() == "DR" && historique.dash(frame) {
                resultat_trial(&mut trial, &["DR".to_string()], "66 (DR)", frame, &partage);
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
                resultat_trial(&mut trial, &candidats, &coup, frame, &partage);
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

/// Donne un coup au trial, l'affiche dans le terminal et met l'overlay à jour.
fn resultat_trial(
    trial: &mut Trial,
    candidats: &[String],
    coup: &str,
    frame: u64,
    partage: &Mutex<Partage>,
) {
    let resultat = trial.coup(candidats, frame);
    let Ok(mut p) = partage.lock() else {
        return;
    };
    p.index = trial.index;
    match resultat {
        Resultat::Avance => println!("{}", affichage::trial(trial)),
        Resultat::Reussi => {
            println!("{}", "      ★ COMBO RÉUSSI ★".bright_green().bold());
            println!("{}", affichage::trial(trial));
            p.reussites += 1;
            p.message("COMBO RÉUSSI !".to_string(), true);
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
