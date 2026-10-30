//! La boucle qui lit la manette, reconnaît les coups et fait avancer le trial.
//!
//! Elle tourne dans son propre thread, pour ne jamais être ralentie par l'affichage.
//! Elle écrit ce qu'elle voit dans le terminal et dans le `Partage` lu par l'overlay.

use crate::affichage::{self, Etat};
use crate::capture::Capture;
use crate::clavier::Touche;
use crate::combo::{Resultat, Trial};
use crate::controles::{Action, Controles};
use crate::ecran::{self, Appui, EntreeJeu};
use crate::horloge;
use crate::manette::{Evenement, Manette};
use crate::motion::Historique;
use crate::notation::Directions;
use crate::partage::{Ligne, Partage};
use crate::timings::{ESSAIS_MIN, Timings};
use colored::Colorize;
use eframe::egui;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Une direction prise moins de 10 ms après un bouton a pu être vue avec lui par le jeu.
const MARGE_DIRECTION: i64 = 100_000;

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
    let mut manette = match Manette::new() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    for nom in manette.noms() {
        println!("Manette détectée : {nom}");
    }
    println!("Ctrl+C pour quitter. Joueur à gauche de l'écran (b = 4 = vers la gauche).\n");
    println!("{}  {}", trial.nom.bold(), affichage::trial(&trial));
    match trial.hits {
        Some(hits) => println!("Combo validé par les hits du jeu : {hits} attendus."),
        None => println!("Hits pas encore appris : appuie sur Select après un vrai combo."),
    }
    println!("F8 : déplacer l'overlay. F9 : garder ou non les vidéos. F10 : photo du jeu (PNG).");
    println!();

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
    let mut debut_temps = horloge::maintenant();
    let mut debut = horloge::frame(debut_temps);
    let mut synchro_annoncee = false;
    let mut nouveaux: Vec<Action> = Vec::new();
    let mut coups: Vec<String> = Vec::new();
    // Dernier essai complet, en attente d'un Select (« c'était un vrai combo ») : frame, écarts.
    let mut a_confirmer: Option<AConfirmer> = None;
    let mut select_appuye = false;
    let mut f8 = Touche::f8();
    let mut f9 = Touche::f9();
    let mut f10 = Touche::f10();
    let mut juge = Juge::default();

    // Le jeu ne lit la manette qu'une fois par frame. On fait pareil : les appuis et les
    // changements d'une frame sont gardés, et traités ensemble quand la frame se termine,
    // avec la direction tenue à la fin de la frame (comme le jeu la voit).
    let mut frame_courante = debut;
    let mut appuis_frame: Vec<(Action, i64)> = Vec::new();
    let mut premier_changement: Option<i64> = None;
    // Appuis d'une frame finie, jugés à la suivante : (action, heure, direction, frame).
    let mut a_juger: Vec<(Action, i64, u8, u64)> = Vec::new();
    let mut dirs_avant = dirs.numpad();
    let mut tenus_avant = tenus.clone();
    // Toutes les directions passées pendant la frame, même moins d'une frame. Le jeu ne lit
    // la manette qu'une fois par frame, et pas forcément au même moment que nous : un ←
    // tenu une demi-frame à la fin d'un 63214 peut être vu par le jeu et pas par nous.
    // Pour reconnaître les mouvements, on les garde toutes.
    let mut directions_frame: Vec<u8> = Vec::new();

    loop {
        // Le numéro de frame, calé sur les images du jeu (voir horloge.rs).
        // Il ne recule jamais, même quand le réglage automatique déplace les limites.
        let temps = horloge::maintenant();
        let frame = horloge::frame(temps).max(frame_courante);
        if !synchro_annoncee && horloge::synchro_active() {
            synchro_annoncee = true;
            println!("{}", "      ✓ frames calées sur les images de SF6".green());
        }
        let mut change = false;

        // 1. La frame d'avant est finie : on traite ce que le jeu y a vu.
        if frame != frame_courante {
            let frame = frame_courante;
            let appuyes: Vec<(Action, i64)> = std::mem::take(&mut appuis_frame);
            let temps_etat = premier_changement.take().unwrap_or(temps);
            let direction = dirs.numpad();
            for d in directions_frame.drain(..) {
                historique.ajouter(frame, d);
            }
            if direction != etat.direction {
                historique.ajouter(frame, direction);
                // Drive Rush par dash (66) : seulement si c'est ce que le trial attend.
                if direction == 6 && trial.attendu() == "DR" && historique.dash(frame) {
                    let fin = resultat_trial(
                        &mut trial,
                        &mut capture,
                        &mut juge,
                        &["DR".to_string()],
                        "66 (DR)",
                        (frame, temps_etat),
                        &partage,
                    );
                    if let Some(fin) = fin {
                        a_confirmer = juge
                            .finir(fin, temps, &trial)
                            .map(|fin| conclure(&trial, &mut capture, fin, frame, &partage));
                    }
                }
            }

            // Les appuis de la frame d'avant : toutes les lectures possibles, puis le trial.
            // Ils sont jugés une frame plus tard : si la direction change juste après
            // l'appui (quelques ms), le jeu a pu voir les deux ensemble (nos limites de frame
            // ne sont pas parfaites). On accepte alors aussi la lecture avec la nouvelle direction.
            for (action, quand, dir_appui, frame_appui) in std::mem::take(&mut a_juger) {
                let mut candidats = lectures(
                    action,
                    dir_appui,
                    frame_appui,
                    &historique,
                    &tenus,
                    &derniers_appuis,
                );
                if direction != dir_appui && temps_etat - quand <= MARGE_DIRECTION {
                    for c in lectures(
                        action,
                        direction,
                        frame,
                        &historique,
                        &tenus,
                        &derniers_appuis,
                    ) {
                        if !candidats.contains(&c) {
                            candidats.push(c);
                        }
                    }
                }
                derniers_appuis.insert(action, frame_appui);
                // On affiche la lecture attendue par le trial si elle y est, sinon la première.
                // Le bouton Drive Parry utilisé pour un Drive Rush attendu s'affiche « DR ».
                let coup = match candidats.iter().find(|c| trial.accepte(c, trial.attendu())) {
                    Some(c) if c == "DP" && trial.attendu() == "DR" => "DR".to_string(),
                    Some(c) => c.clone(),
                    None => candidats[0].clone(),
                };
                // Un nouvel essai commence : le verdict du précédent ne peut plus attendre.
                let nouvel_essai = trial.index == 0
                    && candidats
                        .iter()
                        .any(|c| trial.accepte(c, &trial.etapes[0].notation));
                if nouvel_essai && let Some(fin) = juge.forcer() {
                    a_confirmer = Some(conclure(&trial, &mut capture, fin, frame, &partage));
                }
                let fin = resultat_trial(
                    &mut trial,
                    &mut capture,
                    &mut juge,
                    &candidats,
                    &coup,
                    (frame_appui, quand),
                    &partage,
                );
                if let Some(fin) = fin {
                    a_confirmer = juge
                        .finir(fin, temps, &trial)
                        .map(|fin| conclure(&trial, &mut capture, fin, frame, &partage));
                }
                coups.push(coup);
                change = true;
            }

            // 2. Nouvel état (direction ou boutons différents, ou nouvel appui) :
            //    on affiche l'ancien avec sa durée, puis on démarre le nouveau.
            let nouvel_etat = Etat {
                direction,
                boutons: tenus.iter().copied().collect(),
            };
            if nouvel_etat != etat || !appuyes.is_empty() {
                if frame > debut {
                    // En gris : où l'état a commencé dans sa frame (en ms).
                    println!(
                        "{}  {}",
                        affichage::ligne(&etat, frame - debut, &nouveaux, &coups),
                        format!("@{:.1}", horloge::position_ms(debut_temps)).dimmed()
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
                            debut: debut_temps,
                            exact: false,
                            cassee: false,
                            etape: None,
                        });
                    }
                }

                // 3. Les boutons appuyés : jugés à la frame suivante (voir `a_juger`).
                coups.clear();
                nouveaux.clear();
                for &(action, quand) in &appuyes {
                    a_juger.push((action, quand, direction, frame));
                    if !nouveaux.contains(&action) {
                        nouveaux.push(action);
                    }
                }

                etat = nouvel_etat;
                debut = frame;
                debut_temps = temps_etat;
                change = true;
            }
            frame_courante = horloge::frame(temps).max(frame_courante);
        }

        // 4. On lit TOUS les événements en attente : ils comptent pour la frame en cours.
        for evenement in manette.lire() {
            match evenement {
                Evenement::Appui("Select") => select_appuye = true,
                Evenement::Appui(bouton) => {
                    if !changer_direction(&mut dirs, bouton, true) {
                        match controles.get(bouton) {
                            Some(&action) => {
                                tenus.insert(action);
                                appuis_frame.push((action, temps));
                                juge.appui(temps, action);
                            }
                            None => println!("     bouton {bouton} (pas dans controles.toml)"),
                        }
                    }
                }
                Evenement::Relache(bouton) => {
                    if !changer_direction(&mut dirs, bouton, false)
                        && let Some(action) = controles.get(bouton)
                    {
                        tenus.remove(action);
                    }
                }
                // Stick analogique : transformé en directions avec une zone morte.
                Evenement::StickX(valeur) => {
                    dirs.left = valeur < -0.5;
                    dirs.right = valeur > 0.5;
                }
                Evenement::StickY(valeur) => {
                    dirs.down = valeur < -0.5;
                    dirs.up = valeur > 0.5;
                }
                Evenement::Connectee(nom) => println!("Manette connectée : {nom}"),
                Evenement::Deconnectee => println!("Manette déconnectée"),
            }
        }
        // Heure du premier changement de la frame : début du nouvel état.
        if premier_changement.is_none()
            && (dirs.numpad() != dirs_avant || tenus != tenus_avant || !appuis_frame.is_empty())
        {
            premier_changement = Some(temps);
        }
        if dirs.numpad() != dirs_avant {
            directions_frame.push(dirs.numpad());
        }
        dirs_avant = dirs.numpad();
        tenus_avant = tenus.clone();

        // L'historique du jeu lu à l'écran : ses nouvelles lignes, et le verdict
        // d'un combo fini s'il attendait les frames exactes.
        if juge.lire_journal(temps)
            && let Ok(mut p) = partage.lock()
        {
            p.historique_jeu = juge.visible;
            p.degats_visibles = juge.degats_visibles;
            p.hits_direct = juge.hits_combo;
            juge.corriger(&mut p.lignes);
            juge.publier_lignes(&mut p);
            change = true;
        }
        if let Some(fin) = juge.pret(temps) {
            a_confirmer = Some(conclure(&trial, &mut capture, fin, frame, &partage));
            change = true;
        }
        if f8.appuyee() {
            if let Ok(mut p) = partage.lock() {
                p.deplacement = !p.deplacement;
                let texte = if p.deplacement {
                    "      ✥ déplacement : glisse l'overlay à la souris, F8 pour finir"
                } else {
                    "      ✥ position enregistrée dans overlay.toml"
                };
                println!("{}", texte.cyan());
            }
            change = true;
        }
        if f9.appuyee() {
            capture.enregistre = !capture.enregistre;
            if let Ok(mut p) = partage.lock() {
                p.enregistrement = capture.enregistre;
            }
            let texte = if capture.enregistre {
                "      ● vidéos activées (F9 pour couper)"
            } else {
                "      ○ vidéos coupées (F9 pour réactiver)"
            };
            println!("{}", texte.cyan());
            change = true;
        }
        if f10.appuyee() {
            capture.photo();
        }
        // Les boutons cliquables de l'overlay.
        if let Ok(mut p) = partage.try_lock() {
            if p.enregistrement != capture.enregistre {
                capture.enregistre = p.enregistrement;
                let texte = if capture.enregistre {
                    "      ● vidéos activées"
                } else {
                    "      ○ vidéos coupées"
                };
                println!("{}", texte.cyan());
            }
            if std::mem::take(&mut p.photo_demandee) {
                capture.photo();
            }
        }
        if select_appuye {
            select_appuye = false;
            confirmer(&mut trial, &mut timings, &mut a_confirmer, frame, &partage);
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
/// et dit à la capture vidéo quand filmer. Renvoie le combo fini si c'était la dernière étape
/// (le verdict est donné par `conclure`, éventuellement après lecture de l'historique du jeu).
fn resultat_trial(
    trial: &mut Trial,
    capture: &mut Capture,
    juge: &mut Juge,
    candidats: &[String],
    coup: &str,
    (frame, temps): (u64, i64),
    partage: &Mutex<Partage>,
) -> Option<Fin> {
    let attendue = trial.index;
    let resultat = trial.coup(candidats, frame);
    suivre_video(trial, capture, &resultat, frame);
    // Les appuis des étapes, pour les retrouver ensuite dans l'historique du jeu.
    match resultat {
        Resultat::Avance | Resultat::Rate { .. } if trial.index == 1 => {
            juge.etapes.clear();
            juge.etape(temps, appui_etape(&trial.etapes[0].notation, candidats));
        }
        Resultat::Avance => juge.etape(
            temps,
            appui_etape(&trial.etapes[trial.index - 1].notation, candidats),
        ),
        Resultat::Reussi => {
            if trial.etapes.len() == 1 {
                juge.etapes.clear();
            }
            if let Some(derniere) = trial.etapes.last() {
                juge.etape(temps, appui_etape(&derniere.notation, candidats));
            }
        }
        _ => {}
    }
    let Ok(mut p) = partage.lock() else {
        return None;
    };
    p.index = trial.index;
    p.ecarts = trial.ecarts.clone();
    p.ecarts_exacts = false;
    p.raison_estimee = None;
    if matches!(resultat, Resultat::Avance | Resultat::Reussi)
        && let Some(&ecart) = trial.ecarts.last()
    {
        println!("{}", format!("      +{ecart} f").dimmed());
    }
    match resultat {
        Resultat::Avance => {
            crate::son::etape(trial.index);
            // Une étape passée : l'endroit du dernier break n'est plus affiché.
            p.etape_cassee = None;
            // Son numéro, sur la ligne de l'historique où elle a été faite.
            p.etape_a_venir = Some(trial.index);
            // Nouvel essai : on efface le raté d'avant.
            if trial.index == 1 {
                p.message = None;
            }
            println!("{}", affichage::trial(trial));
        }
        Resultat::Reussi => {
            crate::son::etape(trial.etapes.len());
            p.etape_a_venir = Some(trial.etapes.len());
            return Some(Fin {
                ecarts: trial.ecarts.clone(),
                erreurs: Vec::new(),
                cassee: None,
                temps_cassee: None,
                exact: false,
                raison: None,
                hits: None,
                hits_attendus: None,
            });
        }
        Resultat::Rate { attendu } => {
            println!(
                "{}",
                format!("      ✗ attendu {attendu}, reçu {coup}").red()
            );
            println!("{}", affichage::trial(trial));
            p.message(format!("Raté : attendu {attendu}, reçu {coup}"), false);
            p.etape_cassee = Some(attendue);
            // Le mauvais coup sera sur la prochaine ligne de l'historique.
            p.ligne_cassee_a_venir = true;
        }
        Resultat::Ignore => {}
    }
    None
}

/// Ce qui a été appuyé pour une étape : un Drive Rush fait avec le bouton Drive Parry
/// s'est vu dans le jeu comme MP+MK.
fn appui_etape<'a>(notation: &'a str, candidats: &[String]) -> &'a str {
    if notation == "DR" && candidats.iter().any(|c| c == "DP") {
        "DP"
    } else {
        notation
    }
}

/// Une ligne de l'historique du jeu, pour l'overlay.
/// `debut` : l'heure de l'appui (côté manette), pour la retrouver dans notre historique.
fn ligne_jeu(entree: &EntreeJeu, frames: u64, latence: i64) -> Ligne {
    const BOUTONS: [(u8, Action); 6] = [
        (ecran::LP, Action::LP),
        (ecran::MP, Action::MP),
        (ecran::HP, Action::HP),
        (ecran::LK, Action::LK),
        (ecran::MK, Action::MK),
        (ecran::HK, Action::HK),
    ];
    Ligne {
        frames,
        direction: entree.direction,
        boutons: BOUTONS
            .iter()
            .filter(|(bit, _)| entree.boutons & bit != 0)
            .map(|&(bit, action)| (action, entree.nouveaux & bit != 0))
            .collect(),
        coups: Vec::new(),
        debut: entree.vu - latence,
        exact: true,
        cassee: false,
        etape: None,
    }
}

/// Reprend de notre ligne correspondante (même direction, même moment) les coups reconnus
/// et la marque de break.
fn completer(ligne: &mut Ligne, nos_lignes: &VecDeque<Ligne>) {
    let bits = |l: &Ligne| {
        l.boutons
            .iter()
            .fold(0, |b, (a, _)| b | ecran::boutons_etape(&a.to_string()))
    };
    let boutons = bits(ligne);
    if let Some(notre) = nos_lignes
        .iter()
        .filter(|l| l.direction == ligne.direction && bits(l) == boutons)
        .min_by_key(|l| (l.debut - ligne.debut).abs())
        .filter(|l| (l.debut - ligne.debut).abs() <= 2 * ecran::FRAME)
    {
        ligne.coups = notre.coups.clone();
        ligne.cassee = notre.cassee;
        ligne.etape = notre.etape;
    }
}

/// Un combo fini : écarts entre les étapes et erreurs de timing.
#[derive(Debug, Clone)]
struct Fin {
    ecarts: Vec<u64>,
    erreurs: Vec<String>,
    /// L'étape où le combo a cassé (hors timing, ou après le dernier coup qui a touché).
    cassee: Option<usize>,
    /// L'heure de l'appui de cette étape, pour la retrouver dans l'historique.
    temps_cassee: Option<i64>,
    /// Écarts lus dans l'historique du jeu (sinon : estimés avec notre horloge).
    exact: bool,
    /// Pourquoi les écarts n'ont pas pu être lus dans le jeu.
    raison: Option<String>,
    /// Coups qui ont touché (panneau des dégâts du jeu), et combien il en faut.
    hits: Option<u32>,
    hits_attendus: Option<u32>,
}

/// Donne le verdict d'un combo fini : son, message, vidéo gardée ou non.
/// Renvoie ce que Select pourra confirmer.
fn conclure(
    trial: &Trial,
    capture: &mut Capture,
    fin: Fin,
    frame: u64,
    partage: &Mutex<Partage>,
) -> AConfirmer {
    let ecarts: Vec<String> = fin.ecarts.iter().map(|e| format!("+{e}")).collect();
    let source = if fin.exact {
        "frames exactes du jeu".green()
    } else {
        "frames estimées".yellow()
    };
    println!("      écarts : {} f  ({source})", ecarts.join(" "));
    if let Some(raison) = &fin.raison {
        println!("{}", format!("      frames estimées : {raison}").yellow());
    }
    let hits = match (fin.hits, fin.hits_attendus) {
        (Some(h), Some(m)) => format!(" ({h}/{m} hits)"),
        (Some(h), None) => format!(" ({h} hits)"),
        _ => String::new(),
    };
    let lache = matches!((fin.hits, fin.hits_attendus), (Some(h), Some(m)) if h < m);
    if fin.erreurs.is_empty() {
        println!(
            "{}",
            format!("      ★ COMBO RÉUSSI ★{hits}")
                .bright_green()
                .bold()
        );
        crate::son::reussi();
        capture.reussi();
    } else {
        let erreurs = fin.erreurs.join(", ");
        println!("{}", format!("      ✗ inputs bons, mais {erreurs}").red());
        // Combo lâché ou hors timing : on ne garde pas la vidéo.
        capture.annuler();
    }
    println!("{}", affichage::trial(trial));
    if let Ok(mut p) = partage.lock() {
        p.ecarts = fin.ecarts.clone();
        p.ecarts_exacts = fin.exact;
        p.raison_estimee = fin.raison.clone();
        p.hits = fin.hits;
        p.etape_cassee = if fin.erreurs.is_empty() {
            None
        } else {
            fin.cassee
        };
        if !fin.erreurs.is_empty()
            && let Some(temps) = fin.temps_cassee
        {
            p.marquer_cassee(temps);
        }
        if fin.erreurs.is_empty() {
            p.reussites += 1;
            p.message(format!("COMBO RÉUSSI !{hits}"), true);
        } else if lache {
            p.message(fin.erreurs.join(", "), false);
        } else {
            p.message(format!("Timing : {}", fin.erreurs.join(", ")), false);
        }
    }
    AConfirmer {
        frame,
        ecarts: fin.ecarts,
        hits: fin.hits,
    }
}

/// Le dernier combo fini, que Select peut confirmer (« c'était un vrai combo »).
struct AConfirmer {
    frame: u64,
    ecarts: Vec<u64>,
    hits: Option<u32>,
}

/// Combien de temps on attend l'historique du jeu avant de donner le verdict sans lui.
const ATTENTE_MAX: i64 = 4_000_000; // 400 ms
/// Combien de temps on attend les derniers coups (un Yoga Flame met du temps à toucher).
const ATTENTE_HITS: i64 = 20_000_000; // 2 s
/// Délai d'affichage supposé tant qu'il n'est pas mesuré.
const LATENCE_DEFAUT: i64 = 600_000; // 60 ms

/// Vérifie les combos avec l'historique d'inputs du jeu (frames exactes).
#[derive(Default)]
struct Juge {
    latence: ecran::Latence,
    /// Dernier numéro lu dans le journal du jeu.
    dernier: u64,
    /// Les lignes récentes de l'historique du jeu.
    journal: VecDeque<EntreeJeu>,
    /// Nos appuis récents (heure, bouton), pour mesurer la latence.
    nos_appuis: VecDeque<(i64, u8)>,
    /// Les appuis des étapes de l'essai en cours.
    etapes: Vec<Appui>,
    /// Combo fini qui attend l'historique du jeu.
    attente: Option<Attente>,
    /// L'historique du jeu était lu à la dernière vérification.
    visible: bool,
    /// Dernier numéro lu dans le journal des dégâts.
    dernier_degats: u64,
    /// Heures des coups qui ont touché, et des fins de combo (dégâts revenus à 0).
    coups: VecDeque<i64>,
    fins_combo: VecDeque<i64>,
    /// Le panneau des dégâts du jeu est lu.
    degats_visibles: bool,
    /// Coups qui ont touché dans le combo en cours (ou le dernier), et s'il est fini.
    hits_combo: u32,
    combo_fini: bool,
    /// Lignes du jeu arrivées, pas encore recopiées dans l'historique de l'overlay.
    a_publier: Vec<EntreeJeu>,
    /// La dernière ligne du jeu recopiée (en cours : sa durée n'est pas encore connue).
    en_cours: Option<EntreeJeu>,
}

struct Attente {
    fin: Fin,
    appuis: Vec<Appui>,
    notations: Vec<String>,
    /// Heure du premier et du dernier appui du combo.
    debut: i64,
    quand: i64,
    latence: i64,
    /// Si les frames du jeu ne peuvent pas servir : pourquoi.
    raison_frames: Option<String>,
    hits_attendus: Option<u32>,
}

impl Juge {
    /// Durée gardée pour nos appuis et les lignes du jeu (unités de 100 ns).
    const MEMOIRE: i64 = 50_000_000; // 5 s

    fn appui(&mut self, temps: i64, action: Action) {
        let boutons = ecran::boutons_etape(&action.to_string());
        for i in 0..6 {
            if boutons & (1 << i) != 0 {
                self.nos_appuis.push_back((temps, 1 << i));
            }
        }
        while self
            .nos_appuis
            .front()
            .is_some_and(|(t, _)| temps - t > Self::MEMOIRE)
        {
            self.nos_appuis.pop_front();
        }
    }

    fn etape(&mut self, temps: i64, notation: &str) {
        self.etapes.push(Appui {
            temps,
            boutons: ecran::boutons_etape(notation),
        });
    }

    /// Ajoute les nouvelles lignes de l'historique du jeu et mesure la latence.
    /// Renvoie vrai s'il y a du nouveau (lignes du jeu, historique apparu ou disparu).
    fn lire_journal(&mut self, temps: i64) -> bool {
        let mut nouveau = false;
        let visible = ecran::historique_visible(temps);
        if visible != self.visible {
            self.visible = visible;
            let texte = if visible {
                "      ✓ historique d'inputs de SF6 lu à l'écran : frames exactes".green()
            } else {
                "      ○ historique d'inputs de SF6 plus visible : frames estimées".yellow()
            };
            println!("{texte}");
            nouveau = true;
        }
        let degats = ecran::degats_visibles(temps);
        if degats != self.degats_visibles {
            self.degats_visibles = degats;
            let texte = if degats {
                "      ✓ panneau des dégâts lu : chaque coup qui touche est compté".green()
            } else {
                "      ○ panneau des dégâts plus visible : coups qui touchent non comptés".yellow()
            };
            println!("{texte}");
            nouveau = true;
        }
        for evenement in ecran::nouveaux_degats(&mut self.dernier_degats) {
            match evenement {
                ecran::Degats::Coup(t) => {
                    self.coups.push_back(t);
                    // Compteur en direct : repart de 1 au premier coup d'un nouveau combo
                    // (dégâts revenus à 0, ou plus de 1,5 s sans coup).
                    let pause = self
                        .coups
                        .iter()
                        .rev()
                        .nth(1)
                        .is_some_and(|&p| t - p > 15_000_000);
                    if self.combo_fini || pause {
                        self.hits_combo = 0;
                        self.combo_fini = false;
                    }
                    self.hits_combo += 1;
                }
                ecran::Degats::Fin(t) => {
                    self.fins_combo.push_back(t);
                    self.combo_fini = true;
                }
            }
            nouveau = true;
        }
        for liste in [&mut self.coups, &mut self.fins_combo] {
            while liste.front().is_some_and(|t| temps - t > Self::MEMOIRE) {
                liste.pop_front();
            }
        }
        let appuis: Vec<(i64, u8)> = self.nos_appuis.iter().copied().collect();
        for entree in ecran::nouvelles_entrees(&mut self.dernier) {
            self.latence.mesurer(&entree, &appuis);
            self.journal.push_back(entree);
            self.a_publier.push(entree);
            nouveau = true;
        }
        // Le moment où le jeu lit la manette, déduit de nos appuis : nos limites de frames
        // se calent dessus (un appui juste avant compte pour la frame, juste après pour la
        // suivante, comme dans le jeu).
        if nouveau && let Some(delai) = self.latence.delai_lecture() {
            let premier = !horloge::lecture_calee();
            horloge::caler_lecture(delai);
            if premier {
                println!(
                    "{}",
                    "      ✓ frames calées sur le moment où SF6 lit la manette".green()
                );
            }
        }
        while self
            .journal
            .front()
            .is_some_and(|e| temps - e.vu > Self::MEMOIRE)
        {
            self.journal.pop_front();
        }
        nouveau
    }

    /// Recale la durée des lignes de l'overlay sur celles du jeu : chaque ligne est
    /// retrouvée dans l'historique du jeu (même direction, mêmes boutons, même moment).
    fn corriger(&self, lignes: &mut VecDeque<Ligne>) {
        let Some(latence) = self.latence.valeur() else {
            return;
        };
        let journal: Vec<EntreeJeu> = self.journal.iter().copied().collect();
        // La ligne du jeu de chacune de nos lignes (la plus récente en premier).
        let dans_le_jeu: Vec<Option<usize>> = lignes
            .iter()
            .map(|ligne| {
                let boutons = ligne
                    .boutons
                    .iter()
                    .fold(0, |b, (a, _)| b | ecran::boutons_etape(&a.to_string()));
                ecran::ligne_du_jeu(&journal, ligne.debut + latence, ligne.direction, boutons)
            })
            .collect();
        for (ligne, &i) in lignes.iter_mut().zip(&dans_le_jeu) {
            if !ligne.exact
                && let Some(frames) = i.and_then(|i| ecran::duree_ligne(&journal, i))
            {
                ligne.frames = frames;
                ligne.exact = true;
            }
        }
        // Une ligne à nous que le jeu n'a pas vue : le jeu passe directement de la ligne
        // d'avant à celle d'après. Typiquement une frame de neutre entre ↓ et → sur un
        // leverless : on a lâché ↓ et appuyé → dans deux de nos frames, mais le jeu a lu
        // les deux changements dans la même frame. On l'enlève, comme dans le jeu.
        let absentes: Vec<usize> = (1..lignes.len().saturating_sub(1))
            .filter(|&k| {
                let ligne = &lignes[k];
                dans_le_jeu[k].is_none()
                    && ligne.coups.is_empty()
                    && !ligne.boutons.iter().any(|&(_, nouveau)| nouveau)
                    && matches!(
                        (dans_le_jeu[k + 1], dans_le_jeu[k - 1]),
                        (Some(avant), Some(apres)) if ecran::lignes_consecutives(&journal, avant, apres)
                    )
            })
            .collect();
        for k in absentes.into_iter().rev() {
            lignes.remove(k);
        }
    }

    /// Recopie les nouvelles lignes du jeu dans l'historique de l'overlay, avec les coups
    /// reconnus sur notre ligne correspondante (et sa marque de break).
    fn publier_lignes(&mut self, p: &mut Partage) {
        let latence = self.latence.valeur().unwrap_or(LATENCE_DEFAUT);
        for entree in std::mem::take(&mut self.a_publier) {
            if let Some(avant) = self.en_cours.take() {
                // Une ligne qui change de segment : le jeu a perdu le fil (99 frames...).
                let frames = if avant.segment == entree.segment {
                    entree.frame - avant.frame
                } else {
                    99
                };
                let mut ligne = ligne_jeu(&avant, frames, latence);
                completer(&mut ligne, &p.lignes);
                p.ajouter_ligne_jeu(ligne);
            }
            self.en_cours = Some(entree);
        }
        p.ligne_jeu_en_cours = self.en_cours.map(|e| {
            let mut ligne = ligne_jeu(&e, 0, latence);
            completer(&mut ligne, &p.lignes);
            ligne
        });
    }

    /// Le combo vient de finir. Si l'historique ou les dégâts du jeu sont lus, on attend
    /// leurs informations (`None`, le verdict viendra de `pret`). Sinon, verdict tout de suite.
    fn finir(&mut self, mut fin: Fin, temps: i64, trial: &Trial) -> Option<Fin> {
        if let Some(k) = fin.cassee {
            fin.temps_cassee = self.etapes.get(k).map(|a| a.temps);
        }
        let raison = if !self.visible {
            Some("historique du jeu pas visible")
        } else if self.etapes.len() != trial.etapes.len() {
            Some("étapes mal suivies")
        } else if self.etapes.iter().any(|a| a.boutons == 0) {
            Some("étape sans bouton (DR)")
        } else if self.latence.valeur().is_none() {
            Some("délai du jeu pas encore mesuré (fais quelques appuis isolés)")
        } else {
            None
        }
        .map(str::to_string);
        if raison.is_some() && !self.degats_visibles {
            return Some(Fin { raison, ..fin });
        }
        let appuis = std::mem::take(&mut self.etapes);
        self.attente = Some(Attente {
            fin,
            debut: appuis.first().map_or(temps, |a| a.temps),
            appuis,
            notations: trial.etapes.iter().map(|e| e.notation.clone()).collect(),
            quand: temps,
            latence: self.latence.valeur().unwrap_or(LATENCE_DEFAUT),
            raison_frames: raison,
            hits_attendus: trial.hits,
        });
        None
    }

    /// Le verdict d'un combo en attente, quand le jeu l'a rattrapé (ou quand on a assez attendu) :
    /// historique lu, et tous les coups attendus ont touché (ou le combo est fini).
    fn pret(&mut self, temps: i64) -> Option<Fin> {
        let attente = self.attente.as_ref()?;
        if temps <= attente.quand + attente.latence + 4 * ecran::FRAME {
            return None;
        }
        let exact = self.verifier();
        let frames_finies = exact.is_ok()
            || attente.raison_frames.is_some()
            || temps >= attente.quand + attente.latence + ATTENTE_MAX;
        if !frames_finies {
            return None;
        }
        if self.degats_visibles && temps < attente.quand + ATTENTE_HITS {
            let hits = self.hits_depuis(attente.debut);
            let combo_fini = hits > 0
                && self
                    .fins_combo
                    .iter()
                    .any(|&f| self.coups.back().is_some_and(|&c| f > c) && f > attente.debut);
            // Nombre de hits connu : on attend qu'ils aient tous touché (ou que le combo
            // soit fini). Pas encore appris : on attend la fin du combo pour tous les compter.
            let attendre = match attente.hits_attendus {
                Some(attendus) => hits < attendus && !combo_fini,
                None => !combo_fini,
            };
            if attendre {
                return None;
            }
        }
        self.resultat(exact)
    }

    /// Verdict immédiat (un nouvel essai commence) : avec ce que le jeu a déjà montré.
    fn forcer(&mut self) -> Option<Fin> {
        let exact = self.verifier();
        self.resultat(exact)
    }

    /// Coups qui ont touché depuis `debut`.
    fn hits_depuis(&self, debut: i64) -> u32 {
        self.coups.iter().filter(|&&t| t >= debut).count() as u32
    }

    /// Le verdict : frames exactes si possible (sinon notre estimation, avec la raison),
    /// puis les coups qui ont touché : s'il en manque, le combo a lâché, quel que soit le timing.
    fn resultat(&mut self, exact: Result<Fin, String>) -> Option<Fin> {
        let attente = self.attente.take()?;
        let mut fin = match exact {
            Ok(fin) => fin,
            Err(raison) => {
                if attente.raison_frames.is_none() {
                    self.diagnostic(&attente, &raison);
                }
                Fin {
                    raison: Some(raison),
                    ..attente.fin.clone()
                }
            }
        };
        if self.degats_visibles {
            let hits = self.hits_depuis(attente.debut);
            fin.hits = Some(hits);
            fin.hits_attendus = attente.hits_attendus;
            if let Some(attendus) = attente.hits_attendus {
                if hits >= attendus {
                    // Tous les coups ont touché : le jeu a validé le combo, le timing aussi.
                    fin.erreurs.clear();
                    fin.cassee = None;
                } else {
                    let (erreur, cassee) = self.lache(&attente, hits, attendus);
                    fin.erreurs = vec![erreur];
                    fin.cassee = Some(cassee);
                    fin.temps_cassee = attente.appuis.get(cassee).map(|a| a.temps);
                }
            }
        }
        Some(fin)
    }

    /// « 4/7 hits : lâché après 236MK » : la dernière étape faite avant le dernier coup qui a touché.
    /// Renvoie aussi l'étape d'après, celle où le combo a cassé.
    fn lache(&self, attente: &Attente, hits: u32, attendus: u32) -> (String, usize) {
        let Some(&dernier) = self.coups.iter().rev().find(|&&t| t >= attente.debut) else {
            return (format!("0/{attendus} hits : aucun coup n'a touché"), 0);
        };
        let derniere = attente
            .appuis
            .iter()
            .rposition(|a| a.temps + attente.latence <= dernier);
        let etape = derniere
            .and_then(|k| attente.notations.get(k))
            .map_or("?", String::as_str);
        let cassee = derniere.map_or(0, |k| {
            (k + 1).min(attente.notations.len().saturating_sub(1))
        });
        (
            format!("{hits}/{attendus} hits : lâché après {etape}"),
            cassee,
        )
    }

    /// Les écarts exacts du combo en attente, avec ses erreurs de timing.
    fn verifier(&self) -> Result<Fin, String> {
        let Some(attente) = self.attente.as_ref() else {
            return Err("aucun combo en attente".to_string());
        };
        if let Some(raison) = &attente.raison_frames {
            return Err(raison.clone());
        }
        let journal: Vec<EntreeJeu> = self.journal.iter().copied().collect();
        let ecarts = ecran::ecarts_du_jeu(&attente.appuis, &journal, attente.latence)
            .map_err(|(i, raison)| format!("{} {raison}", attente.notations[i]))?;
        Ok(Fin {
            cassee: None,
            temps_cassee: None,
            ecarts,
            erreurs: Vec::new(),
            exact: true,
            raison: None,
            hits: None,
            hits_attendus: None,
        })
    }

    /// Écrit dans `diagnostic_historique.txt` nos appuis et ce qui a été lu dans le jeu.
    fn diagnostic(&self, attente: &Attente, raison: &str) {
        use std::fmt::Write;
        let debut = attente.appuis.first().map_or(0, |a| a.temps) - 5_000_000;
        let ms = |t: i64| (t - debut) / 10_000;
        let mut t = String::new();
        let _ = writeln!(t, "Combo non vérifié : {raison}");
        let _ = writeln!(t, "Latence mesurée : {} ms", attente.latence / 10_000);
        let _ = writeln!(
            t,
            "\nNos étapes (ms, + latence = moment attendu dans le jeu) :"
        );
        for (a, n) in attente.appuis.iter().zip(&attente.notations) {
            let _ = writeln!(
                t,
                "  {:>6} -> {:>6}  {n} ({})",
                ms(a.temps),
                ms(a.temps + attente.latence),
                ecran::noms_boutons(a.boutons)
            );
        }
        let _ = writeln!(
            t,
            "\nLignes lues dans le jeu (ms, frame, segment, direction, boutons, nouveaux) :"
        );
        for e in self.journal.iter().filter(|e| e.vu >= debut) {
            let _ = writeln!(
                t,
                "  {:>6}  f{:<6} s{:<3} {}  {:<10} {}",
                ms(e.vu),
                e.frame,
                e.segment,
                e.direction,
                ecran::noms_boutons(e.boutons),
                ecran::noms_boutons(e.nouveaux)
            );
        }
        let _ = writeln!(t, "\nNos appuis récents (ms, bouton) :");
        for (temps, b) in self.nos_appuis.iter().filter(|(temps, _)| *temps >= debut) {
            let _ = writeln!(t, "  {:>6}  {}", ms(*temps), ecran::noms_boutons(*b));
        }
        let _ = std::fs::write("diagnostic_historique.txt", t);
    }
}

/// Select après un combo : « c'était un vrai combo ». Ses écarts sont ajoutés aux
/// fenêtres de timing (`timings.toml`).
fn confirmer(
    trial: &mut Trial,
    timings: &mut Timings,
    a_confirmer: &mut Option<AConfirmer>,
    frame: u64,
    partage: &Mutex<Partage>,
) {
    // 10 secondes pour confirmer, après, on ne sait plus de quel essai on parle.
    const DELAI_CONFIRMATION: u64 = 600;
    let texte = match a_confirmer.take() {
        Some(c) if frame.saturating_sub(c.frame) <= DELAI_CONFIRMATION => {
            let n = timings.apprendre(&trial.nom, &c.ecarts);
            if let Some(hits) = c.hits {
                timings.apprendre_hits(&trial.nom, hits);
            }
            if !trial.hits_fixe {
                trial.hits = timings.hits(&trial.nom);
            }
            let hits = trial.hits.map_or(String::new(), |h| format!(", {h} hits"));
            if let Err(e) = timings.enregistrer() {
                eprintln!("{e}");
            }
            if n < ESSAIS_MIN {
                format!("Timing : {n}/{ESSAIS_MIN} vrais combos confirmés{hits}")
            } else {
                format!("Timing appris ({n} vrais combos{hits})")
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
        // Gardée ou non : décidé par `conclure`, une fois les frames du jeu connues.
        Resultat::Reussi => {
            if let Some(derniere) = trial.etapes.last() {
                capture.etape(&derniere.notation, frame);
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
fn changer_direction(dirs: &mut Directions, bouton: &str, pressed: bool) -> bool {
    match bouton {
        "DPadUp" => dirs.up = pressed,
        "DPadDown" => dirs.down = pressed,
        "DPadLeft" => dirs.left = pressed,
        "DPadRight" => dirs.right = pressed,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_ligne_du_jeu_reprend_nos_coups_et_le_break() {
        let entree = EntreeJeu {
            vu: 1_000_000,
            frame: 10,
            segment: 1,
            direction: 4,
            boutons: ecran::MK,
            nouveaux: ecran::MK,
        };
        let latence = 300_000;
        let mut ligne = ligne_jeu(&entree, 4, latence);
        assert_eq!(ligne.boutons, vec![(Action::MK, true)]);
        assert_eq!(ligne.debut, 700_000);

        let notre = |direction, debut, coup: &str| Ligne {
            frames: 5,
            direction,
            boutons: vec![(Action::MK, true)],
            coups: vec![coup.to_string()],
            debut,
            exact: false,
            cassee: true,
            etape: None,
        };
        // Même moment mais autre direction, puis la bonne ligne à une demi-frame près.
        let nos_lignes = VecDeque::from([notre(6, 700_000, "6MK"), notre(4, 780_000, "4MK")]);
        completer(&mut ligne, &nos_lignes);
        assert_eq!(ligne.coups, vec!["4MK".to_string()]);
        assert!(ligne.cassee);

        // Trop loin dans le temps : rien repris.
        let mut seule = ligne_jeu(&entree, 4, latence);
        completer(&mut seule, &VecDeque::from([notre(4, 2_000_000, "4MK")]));
        assert!(seule.coups.is_empty());
    }
}
