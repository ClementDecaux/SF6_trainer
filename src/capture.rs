//! Étape 7 : chaque combo réussi est enregistré en vidéo (MP4, 60 images par seconde).
//!
//! On filme seulement la fenêtre de SF6 (l'overlay n'apparaît donc pas dans la vidéo),
//! avec l'API de capture de Windows (celle qu'utilisent OBS et la Xbox Game Bar).
//! Rien à installer : l'encodage H.264 est fait par Windows lui-même.
//!
//! Le principe :
//! - 1re étape du trial validée : on commence à filmer ;
//! - raté ou trop lent : on jette la vidéo ;
//! - combo réussi : on filme encore un peu (`apres_ms`), puis on garde la vidéo dans
//!   `clips/`, avec un fichier `.json` à côté (combo, personnage, frame de chaque étape).
//!
//! Le thread de la manette envoie des ordres (`Ordre`) au thread de capture par un canal
//! (`mpsc`) : il n'attend jamais la vidéo, les inputs restent lus à la milliseconde.

// Hors Windows, la capture n'existe pas : une partie du code n'y sert pas.
#![cfg_attr(not(windows), allow(dead_code))]

use serde::{Deserialize, Serialize};
use std::sync::mpsc::Sender;

const FICHIER: &str = "capture.toml";

const CONFIG_PAR_DEFAUT: &str = r#"# Enregistrement vidéo des combos réussis (Windows 10 ou 11).
# false = pas de vidéo.
actif = true
# Le programme de SF6, et le titre de sa fenêtre si le programme n'est pas trouvé.
processus = "StreetFighter6.exe"
fenetre = "Street Fighter 6"
# Dossier où les vidéos sont rangées.
dossier = "clips"
# Combien de temps on filme encore après la dernière étape, en millisecondes.
apres_ms = 1500
# Qualité de la vidéo, en mégabits par seconde (20 = très bon en 1080p).
debit_mbps = 20
# Au-delà de cette durée (en secondes), on arrête de filmer et on jette la vidéo.
duree_max_s = 30
"#;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub actif: bool,
    pub processus: String,
    pub fenetre: String,
    pub dossier: String,
    pub apres_ms: u64,
    pub debit_mbps: u32,
    pub duree_max_s: u64,
}

pub fn charger() -> Result<Config, String> {
    let texte = match std::fs::read_to_string(FICHIER) {
        Ok(texte) => texte,
        Err(_) => {
            std::fs::write(FICHIER, CONFIG_PAR_DEFAUT)
                .map_err(|e| format!("impossible de créer {FICHIER} : {e}"))?;
            println!("{FICHIER} créé : mets actif = false pour couper la vidéo.");
            CONFIG_PAR_DEFAUT.to_string()
        }
    };
    toml::from_str(&texte).map_err(|e| format!("erreur dans {FICHIER} : {e}"))
}

/// Ce qui est écrit dans le `.json` à côté de la vidéo.
#[derive(Debug, Clone, Serialize)]
pub struct InfosClip {
    pub personnage: String,
    pub combo: String,
    pub inputs: String,
    /// Date et heure locales, ex. « 2026-10-29 21:03:05 ».
    pub date: String,
    pub fps: u32,
    /// Chaque étape validée, avec sa frame comptée depuis la 1re étape.
    pub etapes: Vec<EtapeClip>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EtapeClip {
    pub notation: String,
    pub frame: u64,
}

/// Les ordres envoyés au thread de capture.
#[derive(Debug)]
pub enum Ordre {
    /// Un essai commence : on filme (une vidéo en cours sans succès est jetée).
    Demarrer,
    /// Combo réussi : on garde la vidéo.
    Garder(InfosClip),
    /// Essai raté : on jette la vidéo.
    Annuler,
}

/// La télécommande du thread de capture, utilisée par le thread de la manette.
/// Elle note aussi la frame de chaque étape pour le `.json`.
/// Si la capture est coupée (ou hors Windows), les ordres ne vont nulle part.
pub struct Capture {
    envoi: Option<Sender<Ordre>>,
    personnage: String,
    combo: String,
    inputs: String,
    debut: u64,
    etapes: Vec<EtapeClip>,
}

impl Capture {
    fn new(envoi: Option<Sender<Ordre>>, personnage: &str, combo: &str, inputs: &str) -> Capture {
        Capture {
            envoi,
            personnage: personnage.to_string(),
            combo: combo.to_string(),
            inputs: inputs.to_string(),
            debut: 0,
            etapes: Vec::new(),
        }
    }

    /// La 1re étape vient d'être validée.
    pub fn debut_essai(&mut self, frame: u64) {
        self.debut = frame;
        self.etapes.clear();
        self.envoyer(Ordre::Demarrer);
    }

    /// Une étape vient d'être validée.
    pub fn etape(&mut self, notation: &str, frame: u64) {
        self.etapes.push(EtapeClip {
            notation: notation.to_string(),
            frame: frame.saturating_sub(self.debut),
        });
    }

    pub fn reussi(&mut self) {
        let infos = InfosClip {
            personnage: self.personnage.clone(),
            combo: self.combo.clone(),
            inputs: self.inputs.clone(),
            date: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            fps: 60,
            etapes: std::mem::take(&mut self.etapes),
        };
        self.envoyer(Ordre::Garder(infos));
    }

    pub fn annuler(&mut self) {
        self.etapes.clear();
        self.envoyer(Ordre::Annuler);
    }

    fn envoyer(&self, ordre: Ordre) {
        if let Some(envoi) = &self.envoi {
            // Si le thread de capture s'est arrêté, tant pis : le trial continue.
            let _ = envoi.send(ordre);
        }
    }
}

/// Démarre le thread de capture (si `actif = true` et sous Windows).
pub fn lancer(config: Config, personnage: &str, combo: &str, inputs: &str) -> Capture {
    if !config.actif {
        println!("Vidéo : coupée dans {FICHIER}.");
        return Capture::new(None, personnage, combo, inputs);
    }
    #[cfg(windows)]
    {
        let (envoi, reception) = std::sync::mpsc::channel();
        std::thread::spawn(move || capture_windows::boucle(config, reception));
        Capture::new(Some(envoi), personnage, combo, inputs)
    }
    #[cfg(not(windows))]
    {
        println!("Vidéo : seulement sous Windows.");
        Capture::new(None, personnage, combo, inputs)
    }
}

/// Le nom des fichiers d'un clip, sans extension :
/// « 2026-10-29_21-03-05_Dhalsim_Drive-Impact-Yoga-Flame ».
pub fn nom_fichier(infos: &InfosClip) -> String {
    let date = infos.date.replace(' ', "_").replace(':', "-");
    format!(
        "{date}_{}_{}",
        nettoyer(&infos.personnage),
        nettoyer(&infos.combo)
    )
}

/// Garde lettres et chiffres, remplace le reste par des tirets (noms de fichiers sûrs).
fn nettoyer(texte: &str) -> String {
    let mut sortie = String::new();
    for c in texte.chars() {
        if c.is_alphanumeric() {
            sortie.push(c);
        } else if !sortie.is_empty() && !sortie.ends_with('-') {
            sortie.push('-');
        }
    }
    sortie.trim_end_matches('-').to_string()
}

/// La partie propre à Windows : la capture elle-même.
#[cfg(windows)]
mod capture_windows {
    use super::{Config, InfosClip, Ordre, nom_fichier};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
    use windows_capture::encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
        VideoSettingsSubType,
    };
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    };
    use windows_capture::window::Window;

    type Erreur = Box<dyn std::error::Error + Send + Sync>;

    /// Cherche SF6, le filme jusqu'à ce que sa fenêtre se ferme, et recommence.
    pub fn boucle(config: Config, ordres: Receiver<Ordre>) {
        let ordres = Arc::new(Mutex::new(ordres));
        let mut prevenu = false;
        // Windows 10 ne sait pas cacher la bordure jaune ni la souris : on réessaie sans.
        let mut options_avancees = true;
        loop {
            // Les ordres reçus pendant que SF6 n'était pas filmé ne servent plus à rien.
            if let Ok(reception) = ordres.lock() {
                reception.try_iter().for_each(drop);
            }
            match trouver_fenetre(&config) {
                Some(fenetre) => {
                    println!("Vidéo : fenêtre de SF6 trouvée, prêt à filmer.");
                    prevenu = false;
                    let (souris, bordure, intervalle) = if options_avancees {
                        (
                            CursorCaptureSettings::WithoutCursor,
                            DrawBorderSettings::WithoutBorder,
                            // Un peu moins de 16,7 ms : on reçoit chaque image du jeu.
                            MinimumUpdateIntervalSettings::Custom(Duration::from_millis(15)),
                        )
                    } else {
                        (
                            CursorCaptureSettings::Default,
                            DrawBorderSettings::Default,
                            MinimumUpdateIntervalSettings::Default,
                        )
                    };
                    let settings = Settings::new(
                        fenetre,
                        souris,
                        bordure,
                        SecondaryWindowSettings::Default,
                        intervalle,
                        DirtyRegionSettings::Default,
                        ColorFormat::Bgra8,
                        Drapeaux {
                            config: config.clone(),
                            ordres: Arc::clone(&ordres),
                        },
                    );
                    if let Err(e) = Enregistreur::start(settings) {
                        let texte = e.to_string();
                        if options_avancees && texte.contains("not supported") {
                            println!(
                                "Vidéo : options avancées non gérées par ce Windows, on fait sans."
                            );
                            options_avancees = false;
                            continue;
                        }
                        eprintln!("Vidéo : capture arrêtée ({texte}).");
                    }
                }
                None if !prevenu => {
                    println!("Vidéo : SF6 n'est pas lancé, je le cherche toutes les 5 secondes.");
                    prevenu = true;
                }
                None => {}
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    }

    /// La fenêtre du programme SF6, sinon une fenêtre au titre exact.
    fn trouver_fenetre(config: &Config) -> Option<Window> {
        let fenetres = Window::enumerate().ok()?;
        fenetres
            .iter()
            .find(|f| {
                f.process_name()
                    .is_ok_and(|nom| nom.eq_ignore_ascii_case(&config.processus))
            })
            .or_else(|| {
                fenetres
                    .iter()
                    .find(|f| f.title().is_ok_and(|t| t == config.fenetre))
            })
            .copied()
    }

    /// Ce que le thread de capture reçoit au démarrage.
    struct Drapeaux {
        config: Config,
        ordres: Arc<Mutex<Receiver<Ordre>>>,
    }

    enum Etat {
        Attente,
        /// On a reçu `Demarrer` : l'encodeur sera créé à la prochaine image (il faut sa taille).
        Demande,
        Filme {
            // `Box` : l'encodeur est gros, on le range à part en mémoire.
            encodeur: Box<VideoEncoder>,
            debut: Instant,
            /// Combo réussi : quand arrêter, et ce qu'on écrira dans le `.json`.
            fin: Option<(Instant, InfosClip)>,
        },
    }

    struct Enregistreur {
        config: Config,
        ordres: Arc<Mutex<Receiver<Ordre>>>,
        etat: Etat,
    }

    impl Enregistreur {
        fn fichier_en_cours(&self) -> PathBuf {
            Path::new(&self.config.dossier).join("en_cours.mp4")
        }

        fn traiter(&mut self, ordre: Ordre) {
            match ordre {
                Ordre::Demarrer => {
                    self.arreter();
                    self.etat = Etat::Demande;
                }
                Ordre::Garder(infos) => match &mut self.etat {
                    Etat::Filme {
                        fin: fin @ None, ..
                    } => {
                        let quand = Instant::now() + Duration::from_millis(self.config.apres_ms);
                        *fin = Some((quand, infos));
                    }
                    Etat::Demande => self.etat = Etat::Attente,
                    _ => {}
                },
                Ordre::Annuler => self.arreter(),
            }
        }

        /// Termine la vidéo en cours : gardée si le combo a été réussi, jetée sinon.
        fn arreter(&mut self) {
            let etat = std::mem::replace(&mut self.etat, Etat::Attente);
            let Etat::Filme { encodeur, fin, .. } = etat else {
                return;
            };
            let en_cours = self.fichier_en_cours();
            if let Err(e) = encodeur.finish() {
                eprintln!("Vidéo : erreur à la fin de l'encodage ({e}).");
                let _ = std::fs::remove_file(&en_cours);
                return;
            }
            match fin {
                Some((_, infos)) => self.ranger(&en_cours, &infos),
                None => {
                    let _ = std::fs::remove_file(&en_cours);
                }
            }
        }

        /// Renomme la vidéo et écrit le `.json` à côté.
        fn ranger(&self, en_cours: &Path, infos: &InfosClip) {
            let base = Path::new(&self.config.dossier).join(nom_fichier(infos));
            let video = base.with_extension("mp4");
            if let Err(e) = std::fs::rename(en_cours, &video) {
                eprintln!("Vidéo : impossible de ranger le clip ({e}).");
                return;
            }
            match serde_json::to_string_pretty(infos) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(base.with_extension("json"), json) {
                        eprintln!("Vidéo : impossible d'écrire le .json ({e}).");
                    }
                }
                Err(e) => eprintln!("Vidéo : impossible de préparer le .json ({e})."),
            }
            println!("      🎬 clip enregistré : {}", video.display());
        }

        fn creer_encodeur(&self, largeur: u32, hauteur: u32) -> Result<VideoEncoder, Erreur> {
            std::fs::create_dir_all(&self.config.dossier)?;
            let video = VideoSettingsBuilder::new(largeur, hauteur)
                .sub_type(VideoSettingsSubType::H264)
                .frame_rate(60)
                .bitrate(self.config.debit_mbps.saturating_mul(1_000_000));
            let encodeur = VideoEncoder::new(
                video,
                AudioSettingsBuilder::default().disabled(true),
                ContainerSettingsBuilder::default(),
                self.fichier_en_cours(),
            )?;
            Ok(encodeur)
        }
    }

    impl GraphicsCaptureApiHandler for Enregistreur {
        type Flags = Drapeaux;
        type Error = Erreur;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Enregistreur {
                config: ctx.flags.config,
                ordres: ctx.flags.ordres,
                etat: Etat::Attente,
            })
        }

        /// Appelé à chaque nouvelle image du jeu (60 fois par seconde).
        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            _controle: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            // 1. Les ordres arrivés depuis la dernière image.
            let ordres: Vec<Ordre> = match self.ordres.lock() {
                Ok(reception) => reception.try_iter().collect(),
                Err(_) => Vec::new(),
            };
            for ordre in ordres {
                self.traiter(ordre);
            }

            // 2. Un essai commence : on crée l'encodeur à la taille de la fenêtre.
            if matches!(self.etat, Etat::Demande) {
                self.etat = match self.creer_encodeur(frame.width(), frame.height()) {
                    Ok(encodeur) => Etat::Filme {
                        encodeur: Box::new(encodeur),
                        debut: Instant::now(),
                        fin: None,
                    },
                    Err(e) => {
                        eprintln!("Vidéo : impossible de démarrer l'encodage ({e}).");
                        Etat::Attente
                    }
                };
            }

            // 3. On ajoute l'image à la vidéo, et on regarde s'il est temps d'arrêter.
            let mut terminer = false;
            if let Etat::Filme {
                encodeur,
                debut,
                fin,
            } = &mut self.etat
            {
                if let Err(e) = encodeur.send_frame(frame) {
                    eprintln!("Vidéo : image perdue ({e}).");
                }
                let maintenant = Instant::now();
                match fin {
                    Some((quand, _)) => terminer = maintenant >= *quand,
                    None => {
                        if debut.elapsed() > Duration::from_secs(self.config.duree_max_s) {
                            // Rien de réussi depuis trop longtemps : on jette.
                            terminer = true;
                        }
                    }
                }
            }
            if terminer {
                self.arreter();
            }
            Ok(())
        }

        /// SF6 a été fermé : la vidéo en cours est terminée (gardée si réussie).
        fn on_closed(&mut self) -> Result<(), Self::Error> {
            self.arreter();
            println!("Vidéo : fenêtre de SF6 fermée.");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn capture_de_test() -> (Capture, mpsc::Receiver<Ordre>) {
        let (envoi, reception) = mpsc::channel();
        let capture = Capture::new(
            Some(envoi),
            "Dhalsim",
            "Drive Impact, Yoga Flame",
            "b+hp xx DI",
        );
        (capture, reception)
    }

    #[test]
    fn frames_comptees_depuis_la_premiere_etape() {
        let (mut capture, reception) = capture_de_test();
        capture.debut_essai(1000);
        capture.etape("4HP", 1000);
        capture.etape("DI", 1012);
        capture.reussi();

        assert!(matches!(reception.try_recv(), Ok(Ordre::Demarrer)));
        let Ok(Ordre::Garder(infos)) = reception.try_recv() else {
            panic!("on attendait Garder");
        };
        assert_eq!(infos.personnage, "Dhalsim");
        assert_eq!(
            infos.etapes,
            vec![
                EtapeClip {
                    notation: "4HP".into(),
                    frame: 0
                },
                EtapeClip {
                    notation: "DI".into(),
                    frame: 12
                },
            ]
        );
    }

    #[test]
    fn nouvel_essai_efface_les_etapes() {
        let (mut capture, _reception) = capture_de_test();
        capture.debut_essai(10);
        capture.etape("4HP", 10);
        capture.debut_essai(500);
        capture.etape("4HP", 500);
        assert_eq!(capture.etapes.len(), 1);
        assert_eq!(capture.etapes[0].frame, 0);
    }

    #[test]
    fn nom_de_fichier_sans_caracteres_speciaux() {
        let infos = InfosClip {
            personnage: "Dhalsim".into(),
            combo: "Drive Impact, Yoga Flame !".into(),
            inputs: String::new(),
            date: "2026-10-29 21:03:05".into(),
            fps: 60,
            etapes: vec![],
        };
        assert_eq!(
            nom_fichier(&infos),
            "2026-10-29_21-03-05_Dhalsim_Drive-Impact-Yoga-Flame"
        );
    }

    #[test]
    fn config_par_defaut_valide() {
        let config: Config = toml::from_str(CONFIG_PAR_DEFAUT).unwrap();
        assert!(config.actif);
        assert_eq!(config.processus, "StreetFighter6.exe");
    }
}
