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
use crate::timings::{ESSAIS_MIN, Timings, verifier};
use colored::Colorize;
use eframe::egui;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    match &trial.fenetres {
        Some(_) => println!("Timing : fenêtres apprises dans timings.toml."),
        None => println!(
            "Timing : pas encore appris. Appuie sur Select après un vrai combo ({ESSAIS_MIN} fois)."
        ),
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
    let mut a_confirmer: Option<(u64, Vec<u64>)> = None;
    let mut select_appuye = false;
    let mut f8 = Touche::f8();
    let mut f9 = Touche::f9();
    let mut f10 = Touche::f10();
    let mut juge = Juge::default();

    loop {
        // 1. On lit TOUS les événements en attente avant de calculer quoi que ce soit :
        //    une manette envoie souvent plusieurs changements en même temps.
        let mut appuyes: Vec<Action> = Vec::new();
        for evenement in manette.lire() {
            match evenement {
                Evenement::Appui("Select") => select_appuye = true,
                Evenement::Appui(bouton) => {
                    if !changer_direction(&mut dirs, bouton, true) {
                        match controles.get(bouton) {
                            Some(&action) => {
                                tenus.insert(action);
                                appuyes.push(action);
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

        // Le numéro de frame, calé sur les images du jeu (voir horloge.rs).
        let temps = horloge::maintenant();
        let frame = horloge::frame(temps);
        if !synchro_annoncee && horloge::synchro_active() {
            synchro_annoncee = true;
            println!("{}", "      ✓ frames calées sur les images de SF6".green());
        }
        let mut change = false;

        // L'historique du jeu lu à l'écran : nos appuis, ses nouvelles lignes, et le verdict
        // d'un combo fini s'il attendait les frames exactes.
        for &action in &appuyes {
            juge.appui(temps, action);
        }
        if juge.lire_journal(temps)
            && let Ok(mut p) = partage.lock()
        {
            p.historique_jeu = juge.visible;
            juge.corriger(&mut p.lignes);
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
        if select_appuye {
            select_appuye = false;
            confirmer(&mut trial, &mut timings, &mut a_confirmer, frame, &partage);
            change = true;
        }
        let direction = dirs.numpad();
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
                    (frame, temps),
                    &partage,
                );
                if let Some(fin) = fin {
                    a_confirmer = juge
                        .finir(fin, temps, &trial)
                        .map(|fin| conclure(&trial, &mut capture, fin, frame, &partage));
                }
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
                // En gris : où l'état a commencé dans sa frame (en ms), pour régler
                // `decalage_ms` si une durée diffère d'une frame avec le jeu.
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
                // Un nouvel essai commence : le verdict du précédent ne peut plus attendre.
                let nouvel_essai =
                    trial.index == 0 && candidats.iter().any(|c| *c == trial.etapes[0].notation);
                if nouvel_essai && let Some(fin) = juge.forcer() {
                    a_confirmer = Some(conclure(&trial, &mut capture, fin, frame, &partage));
                }
                let fin = resultat_trial(
                    &mut trial,
                    &mut capture,
                    &mut juge,
                    &candidats,
                    &coup,
                    (frame, temps),
                    &partage,
                );
                if let Some(fin) = fin {
                    a_confirmer = juge
                        .finir(fin, temps, &trial)
                        .map(|fin| conclure(&trial, &mut capture, fin, frame, &partage));
                }
                coups.push(coup);
            }

            etat = nouvel_etat;
            debut = frame;
            debut_temps = temps;
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
    let resultat = trial.coup(candidats, frame);
    suivre_video(trial, capture, &resultat, frame);
    // Les appuis des étapes, pour les retrouver ensuite dans l'historique du jeu.
    match resultat {
        Resultat::Avance | Resultat::Rate { .. } if trial.index == 1 => {
            juge.etapes.clear();
            juge.etape(temps, &trial.etapes[0].notation);
        }
        Resultat::Avance => juge.etape(temps, &trial.etapes[trial.index - 1].notation),
        Resultat::Reussi => {
            if trial.etapes.len() == 1 {
                juge.etapes.clear();
            }
            if let Some(derniere) = trial.etapes.last() {
                juge.etape(temps, &derniere.notation);
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
        Resultat::Avance => println!("{}", affichage::trial(trial)),
        Resultat::Reussi => {
            return Some(Fin {
                ecarts: trial.ecarts.clone(),
                erreurs: trial.erreurs_timing.clone(),
                exact: false,
                raison: None,
            });
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
    None
}

/// Un combo fini : écarts entre les étapes et erreurs de timing.
#[derive(Debug, Clone)]
struct Fin {
    ecarts: Vec<u64>,
    erreurs: Vec<String>,
    /// Écarts lus dans l'historique du jeu (sinon : estimés avec notre horloge).
    exact: bool,
    /// Pourquoi les écarts n'ont pas pu être lus dans le jeu.
    raison: Option<String>,
}

/// Donne le verdict d'un combo fini : son, message, vidéo gardée ou non.
/// Renvoie ce que Select pourra confirmer (frame, écarts).
fn conclure(
    trial: &Trial,
    capture: &mut Capture,
    fin: Fin,
    frame: u64,
    partage: &Mutex<Partage>,
) -> (u64, Vec<u64>) {
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
    if fin.erreurs.is_empty() {
        println!("{}", "      ★ COMBO RÉUSSI ★".bright_green().bold());
        crate::son::reussi();
        capture.reussi();
    } else {
        let erreurs = fin.erreurs.join(", ");
        println!("{}", format!("      ✗ inputs bons, mais {erreurs}").red());
        // Hors timing : le combo n'est sûrement pas passé, on ne garde pas la vidéo.
        capture.annuler();
    }
    println!("{}", affichage::trial(trial));
    if let Ok(mut p) = partage.lock() {
        p.ecarts = fin.ecarts.clone();
        p.ecarts_exacts = fin.exact;
        p.raison_estimee = fin.raison.clone();
        if fin.erreurs.is_empty() {
            p.reussites += 1;
            p.message("COMBO RÉUSSI !".to_string(), true);
        } else {
            p.message(format!("Timing : {}", fin.erreurs.join(", ")), false);
        }
    }
    (frame, fin.ecarts)
}

/// Combien de temps on attend l'historique du jeu avant de donner le verdict sans lui.
const ATTENTE_MAX: i64 = 4_000_000; // 400 ms

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
}

struct Attente {
    fin: Fin,
    appuis: Vec<Appui>,
    fenetres: Option<Vec<(u64, u64)>>,
    notations: Vec<String>,
    /// Heure du dernier appui du combo.
    quand: i64,
    latence: i64,
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
        let appuis: Vec<(i64, u8)> = self.nos_appuis.iter().copied().collect();
        for entree in ecran::nouvelles_entrees(&mut self.dernier) {
            self.latence.mesurer(&entree, &appuis);
            self.journal.push_back(entree);
            nouveau = true;
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
        for ligne in lignes.iter_mut().filter(|l| !l.exact) {
            let boutons = ligne
                .boutons
                .iter()
                .fold(0, |b, (a, _)| b | ecran::boutons_etape(&a.to_string()));
            if let Some(frames) =
                ecran::duree_dans_le_jeu(&journal, ligne.debut + latence, ligne.direction, boutons)
            {
                ligne.frames = frames;
                ligne.exact = true;
            }
        }
    }

    /// Le combo vient de finir. Si l'historique du jeu est lu, on attend ses frames exactes
    /// (`None`, le verdict viendra de `pret`). Sinon, verdict tout de suite avec notre estimation.
    fn finir(&mut self, fin: Fin, temps: i64, trial: &Trial) -> Option<Fin> {
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
        };
        match (raison, self.latence.valeur()) {
            (None, Some(latence)) => {
                self.attente = Some(Attente {
                    fin,
                    appuis: std::mem::take(&mut self.etapes),
                    fenetres: trial.fenetres.clone(),
                    notations: trial.etapes.iter().map(|e| e.notation.clone()).collect(),
                    quand: temps,
                    latence,
                });
                None
            }
            (raison, _) => Some(Fin {
                raison: raison.map(str::to_string),
                ..fin
            }),
        }
    }

    /// Le verdict d'un combo en attente, quand l'historique du jeu l'a rattrapé
    /// (ou quand on a assez attendu).
    fn pret(&mut self, temps: i64) -> Option<Fin> {
        let attente = self.attente.as_ref()?;
        let affiche = temps > attente.quand + attente.latence + 4 * ecran::FRAME;
        if !affiche {
            return None;
        }
        let exact = self.verifier();
        if exact.is_err() && temps < attente.quand + attente.latence + ATTENTE_MAX {
            return None;
        }
        self.resultat(exact)
    }

    /// Verdict immédiat (un nouvel essai commence) : exact si l'historique est déjà là.
    fn forcer(&mut self) -> Option<Fin> {
        let exact = self.verifier();
        self.resultat(exact)
    }

    /// Le verdict : exact si possible, sinon notre estimation avec la raison
    /// (et un fichier de diagnostic pour comprendre).
    fn resultat(&mut self, exact: Result<Fin, String>) -> Option<Fin> {
        let attente = self.attente.take()?;
        match exact {
            Ok(fin) => Some(fin),
            Err(raison) => {
                self.diagnostic(&attente, &raison);
                Some(Fin {
                    raison: Some(raison),
                    ..attente.fin
                })
            }
        }
    }

    /// Les écarts exacts du combo en attente, avec ses erreurs de timing.
    fn verifier(&self) -> Result<Fin, String> {
        let Some(attente) = self.attente.as_ref() else {
            return Err("aucun combo en attente".to_string());
        };
        let journal: Vec<EntreeJeu> = self.journal.iter().copied().collect();
        let ecarts = ecran::ecarts_du_jeu(&attente.appuis, &journal, attente.latence)
            .map_err(|(i, raison)| format!("{} {raison}", attente.notations[i]))?;
        let mut erreurs = Vec::new();
        if let Some(fenetres) = &attente.fenetres {
            for (i, (&ecart, &fenetre)) in ecarts.iter().zip(fenetres).enumerate() {
                if let Some(e) = verifier(ecart, fenetre) {
                    erreurs.push(format!("{} {e}", attente.notations[i + 1]));
                }
            }
        }
        Ok(Fin {
            ecarts,
            erreurs,
            exact: true,
            raison: None,
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
