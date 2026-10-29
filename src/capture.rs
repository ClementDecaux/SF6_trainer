//! Étape 7 : chaque combo réussi est enregistré en vidéo (MP4, 60 images par seconde).
//!
//! On filme seulement la fenêtre de SF6 (l'overlay n'apparaît donc pas dans la vidéo),
//! avec l'API de capture de Windows (celle qu'utilisent OBS et la Xbox Game Bar).
//! Rien à installer : l'encodage H.264 est fait par Windows lui-même.
//!
//! Le principe :
//! - on garde toujours en mémoire les dernières images (`avant_ms`) ;
//! - 1re étape du trial validée : la vidéo commence par ces images, puis on filme ;
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
# false = pas de capture du jeu (ni vidéo, ni frames calées sur le jeu).
actif = true
# Garder les vidéos dès le lancement ? Sinon, F9 en jeu pour les activer ou les couper.
enregistrer = false
# Réglage fin des frames, en millisecondes (de -16 à 16). Si l'outil compte encore une
# frame de trop ou de moins par rapport à l'historique du jeu, on le règle ici.
decalage_ms = 0.0
# Lire l'historique d'inputs affiché par SF6 (à gauche de l'écran) pour avoir les durées
# exactes du jeu. Active l'affichage des inputs dans le mode entraînement.
historique = true
# Le programme de SF6, et le titre de sa fenêtre si le programme n'est pas trouvé.
processus = "StreetFighter6.exe"
fenetre = "Street Fighter 6"
# Dossier où les vidéos sont rangées.
dossier = "clips"
# Combien de temps on filme avant la 1re étape, en millisecondes.
# Ces images sont gardées en mémoire : environ 350 Mo pour 700 ms en 1080p. 0 = rien avant.
avant_ms = 700
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
    /// Absent des anciens fichiers : `false` (F9 pour activer).
    #[serde(default)]
    pub enregistrer: bool,
    /// Absent des anciens fichiers : 0.
    #[serde(default)]
    pub decalage_ms: f64,
    /// Absent des anciens fichiers : `true`.
    #[serde(default = "vrai")]
    pub historique: bool,
    pub processus: String,
    pub fenetre: String,
    pub dossier: String,
    /// Absent des anciens fichiers : 700 par défaut.
    #[serde(default = "avant_ms_par_defaut")]
    pub avant_ms: u64,
    pub apres_ms: u64,
    pub debit_mbps: u32,
    pub duree_max_s: u64,
}

fn vrai() -> bool {
    true
}

fn avant_ms_par_defaut() -> u64 {
    700
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
    /// F10 : enregistre la prochaine image du jeu en PNG (pleine résolution).
    Photo,
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
    /// Vidéos gardées ou non (F9). Coupé, le jeu reste capté pour caler les frames.
    pub enregistre: bool,
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
            enregistre: false,
        }
    }

    /// La 1re étape vient d'être validée.
    pub fn debut_essai(&mut self, frame: u64) {
        self.debut = frame;
        self.etapes.clear();
        if self.enregistre {
            self.envoyer(Ordre::Demarrer);
        }
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

    /// Une capture d'écran du jeu (PNG) dans le dossier des clips.
    pub fn photo(&mut self) {
        self.envoyer(Ordre::Photo);
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
    crate::horloge::regler_decalage(config.decalage_ms);
    if !config.actif {
        println!("Vidéo : coupée dans {FICHIER}.");
        return Capture::new(None, personnage, combo, inputs);
    }
    #[cfg(windows)]
    {
        let (envoi, reception) = std::sync::mpsc::channel();
        let enregistre = config.enregistrer;
        std::thread::spawn(move || capture_windows::boucle(config, reception));
        let mut capture = Capture::new(Some(envoi), personnage, combo, inputs);
        capture.enregistre = enregistre;
        capture
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

/// Une image du jeu gardée en mémoire (pixels BGRA, lignes du bas vers le haut).
pub struct Image {
    /// Heure de l'image, en unités de 100 ns (horloge de Windows).
    pub temps: i64,
    pub largeur: u32,
    pub hauteur: u32,
    pub pixels: Vec<u8>,
}

/// Les dernières images du jeu (`avant_ms`), pour que la vidéo commence
/// un peu avant la 1re étape au lieu de couper le début du combo.
pub struct Tampon {
    images: std::collections::VecDeque<Image>,
    /// Mémoire des images jetées, réutilisée pour les suivantes (évite d'en redemander).
    libres: Vec<Vec<u8>>,
    /// Durée gardée, en unités de 100 ns.
    duree: i64,
    octets_max: usize,
}

/// Au-delà, on garde moins d'images (utile en 4K).
const OCTETS_MAX_TAMPON: usize = 1_000_000_000;

impl Tampon {
    pub fn new(avant_ms: u64, octets_max: usize) -> Tampon {
        Tampon {
            images: std::collections::VecDeque::new(),
            libres: Vec::new(),
            duree: avant_ms as i64 * 10_000,
            octets_max,
        }
    }

    pub fn actif(&self) -> bool {
        self.duree > 0
    }

    /// De la mémoire pour une nouvelle image (réutilisée si possible).
    pub fn place(&mut self) -> Vec<u8> {
        self.libres.pop().unwrap_or_default()
    }

    pub fn ajouter(&mut self, image: Image) {
        let temps = image.temps;
        self.images.push_back(image);
        let mut octets: usize = self.images.iter().map(|i| i.pixels.len()).sum();
        while let Some(plus_vieille) = self.images.front() {
            if temps - plus_vieille.temps <= self.duree && octets <= self.octets_max {
                break;
            }
            octets -= plus_vieille.pixels.len();
            if let Some(image) = self.images.pop_front() {
                self.recycler(image.pixels);
            }
        }
    }

    /// Toutes les images gardées, de la plus vieille à la plus récente. Le tampon est vidé.
    pub fn prendre(&mut self) -> Vec<Image> {
        self.images.drain(..).collect()
    }

    pub fn recycler(&mut self, pixels: Vec<u8>) {
        // Deux suffisent : on en reprend une à chaque image.
        if self.libres.len() < 2 {
            self.libres.push(pixels);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.images.len()
    }
}

/// Copie une image en retournant ses lignes : Windows encode les pixels donnés « à la main »
/// en commençant par la ligne du bas. `pas` = octets par ligne dans `source` (parfois plus
/// que la largeur, la carte graphique ajoute du remplissage).
pub fn retourner_lignes(
    source: &[u8],
    pas: usize,
    octets_par_ligne: usize,
    hauteur: usize,
    destination: &mut Vec<u8>,
) {
    destination.resize(octets_par_ligne * hauteur, 0);
    for y in 0..hauteur {
        let Some(ligne) = source.get(y * pas..y * pas + octets_par_ligne) else {
            break;
        };
        let cible = (hauteur - 1 - y) * octets_par_ligne;
        destination[cible..cible + octets_par_ligne].copy_from_slice(ligne);
    }
}

/// La partie propre à Windows : la capture elle-même.
#[cfg(windows)]
mod capture_windows {
    use super::{
        Config, Image, InfosClip, OCTETS_MAX_TAMPON, Ordre, Tampon, nom_fichier, retourner_lignes,
    };
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
    use windows_capture::encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, ImageFormat, VideoEncoder,
        VideoSettingsBuilder, VideoSettingsSubType,
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
            nettoyer_dossier(&config.dossier);
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

    /// Supprime les vidéos `en_cours_*` laissées par un arrêt brutal.
    fn nettoyer_dossier(dossier: &str) {
        let Ok(fichiers) = std::fs::read_dir(dossier) else {
            return;
        };
        for fichier in fichiers.flatten() {
            if fichier
                .file_name()
                .to_string_lossy()
                .starts_with("en_cours")
            {
                let _ = std::fs::remove_file(fichier.path());
            }
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

    /// Un encodeur créé à l'avance : le créer prend un moment, et pendant ce temps
    /// aucune image n'arrive. Préparé avant la 1re étape, il démarre tout de suite.
    struct Pret {
        encodeur: VideoEncoder,
        chemin: PathBuf,
        largeur: u32,
        hauteur: u32,
    }

    enum Etat {
        Attente,
        /// On a reçu `Demarrer` : on commence à la prochaine image.
        Demande,
        Filme {
            // `Box` : l'encodeur est gros, on le range à part en mémoire.
            encodeur: Box<VideoEncoder>,
            chemin: PathBuf,
            debut: Instant,
            /// Combo réussi : quand arrêter, et ce qu'on écrira dans le `.json`.
            fin: Option<(Instant, InfosClip)>,
        },
    }

    struct Enregistreur {
        config: Config,
        ordres: Arc<Mutex<Receiver<Ordre>>>,
        etat: Etat,
        pret: Option<Box<Pret>>,
        /// Si la préparation a échoué, on ne réessaie qu'au prochain essai (pas à chaque image).
        echec_preparation: bool,
        tampon: Tampon,
        /// Pour donner un nom différent à chaque vidéo en cours.
        numero: u32,
        /// F10 appuyée : la prochaine image est enregistrée en PNG.
        photo: bool,
        /// Lecture de l'historique d'inputs du jeu.
        suivi: crate::ecran::Suivi,
    }

    impl Enregistreur {
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
                Ordre::Photo => self.photo = true,
            }
        }

        /// Termine la vidéo en cours : gardée si le combo a été réussi, jetée sinon.
        fn arreter(&mut self) {
            let etat = std::mem::replace(&mut self.etat, Etat::Attente);
            let Etat::Filme {
                encodeur,
                chemin,
                fin,
                ..
            } = etat
            else {
                return;
            };
            if let Err(e) = encodeur.finish() {
                eprintln!("Vidéo : erreur à la fin de l'encodage ({e}).");
                let _ = std::fs::remove_file(&chemin);
                return;
            }
            match fin {
                Some((_, infos)) => self.ranger(&chemin, &infos),
                None => {
                    let _ = std::fs::remove_file(&chemin);
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

        fn creer_encodeur(&mut self, largeur: u32, hauteur: u32) -> Result<Pret, Erreur> {
            std::fs::create_dir_all(&self.config.dossier)?;
            self.numero += 1;
            let chemin =
                Path::new(&self.config.dossier).join(format!("en_cours_{}.mp4", self.numero));
            let video = VideoSettingsBuilder::new(largeur, hauteur)
                .sub_type(VideoSettingsSubType::H264)
                .frame_rate(60)
                .bitrate(self.config.debit_mbps.saturating_mul(1_000_000));
            let encodeur = VideoEncoder::new(
                video,
                AudioSettingsBuilder::default().disabled(true),
                ContainerSettingsBuilder::default(),
                &chemin,
            )?;
            Ok(Pret {
                encodeur,
                chemin,
                largeur,
                hauteur,
            })
        }

        /// Un encodeur à la bonne taille : celui préparé à l'avance, sinon un nouveau.
        fn prendre_encodeur(&mut self, largeur: u32, hauteur: u32) -> Result<Pret, Erreur> {
            match self.pret.take() {
                Some(pret) if pret.largeur == largeur && pret.hauteur == hauteur => Ok(*pret),
                autre => {
                    if let Some(pret) = autre {
                        jeter(*pret);
                    }
                    self.creer_encodeur(largeur, hauteur)
                }
            }
        }

        /// Lit l'historique d'inputs du jeu sur l'image, et la copie dans le tampon
        /// (pour filmer un peu avant la 1re étape).
        fn lire_image(&mut self, frame: &mut Frame, temps: i64) -> Result<(), Erreur> {
            let (largeur, hauteur) = (frame.width(), frame.height());
            let mut image = frame.buffer()?;
            let pas = image.row_pitch() as usize;
            let brut = image.as_raw_buffer();
            if self.config.historique {
                let vue = crate::ecran::Vue::jeu(brut, largeur as usize, hauteur as usize, pas);
                crate::ecran::lire_image(&mut self.suivi, &vue, temps);
            }
            if !self.tampon.actif() {
                return Ok(());
            }
            let mut pixels = self.tampon.place();
            retourner_lignes(
                brut,
                pas,
                largeur as usize * 4,
                hauteur as usize,
                &mut pixels,
            );
            self.tampon.ajouter(Image {
                temps,
                largeur,
                hauteur,
                pixels,
            });
            Ok(())
        }

        /// Démarre la vidéo : d'abord les images du tampon, puis l'image actuelle.
        fn commencer(&mut self, frame: &Frame, temps: i64) {
            let (largeur, hauteur) = (frame.width(), frame.height());
            let Pret {
                mut encodeur,
                chemin,
                ..
            } = match self.prendre_encodeur(largeur, hauteur) {
                Ok(pret) => pret,
                Err(e) => {
                    eprintln!("Vidéo : impossible de démarrer l'encodage ({e}).");
                    self.etat = Etat::Attente;
                    return;
                }
            };
            let mut image_actuelle_envoyee = false;
            for image in self.tampon.prendre() {
                if image.largeur == largeur && image.hauteur == hauteur {
                    if let Err(e) = encodeur.send_frame_buffer(&image.pixels, image.temps) {
                        eprintln!("Vidéo : image perdue ({e}).");
                    }
                    image_actuelle_envoyee |= image.temps == temps;
                }
                self.tampon.recycler(image.pixels);
            }
            if !image_actuelle_envoyee && let Err(e) = encodeur.send_frame(frame) {
                eprintln!("Vidéo : image perdue ({e}).");
            }
            self.etat = Etat::Filme {
                encodeur: Box::new(encodeur),
                chemin,
                debut: Instant::now(),
                fin: None,
            };
        }
    }

    /// Ferme un encodeur qui n'a pas servi et supprime son fichier.
    fn jeter(pret: Pret) {
        let _ = pret.encodeur.finish();
        let _ = std::fs::remove_file(&pret.chemin);
    }

    impl GraphicsCaptureApiHandler for Enregistreur {
        type Flags = Drapeaux;
        type Error = Erreur;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            let tampon = Tampon::new(ctx.flags.config.avant_ms, OCTETS_MAX_TAMPON);
            Ok(Enregistreur {
                config: ctx.flags.config,
                ordres: ctx.flags.ordres,
                etat: Etat::Attente,
                pret: None,
                echec_preparation: false,
                tampon,
                numero: 0,
                photo: false,
                suivi: crate::ecran::Suivi::default(),
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
                if matches!(ordre, Ordre::Demarrer) {
                    self.echec_preparation = false;
                }
                self.traiter(ordre);
            }

            if self.photo {
                self.photo = false;
                let nom = format!(
                    "photo_{}.png",
                    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
                );
                let chemin = Path::new(&self.config.dossier).join(nom);
                let _ = std::fs::create_dir_all(&self.config.dossier);
                match frame.save_as_image(&chemin, ImageFormat::Png) {
                    Ok(()) => println!("      📷 photo du jeu : {}", chemin.display()),
                    Err(e) => eprintln!("Vidéo : photo impossible ({e})."),
                }
            }

            // 2. L'image est lue (historique du jeu) et va dans le tampon des dernières images.
            let temps = frame.timestamp()?.Duration;
            // L'heure de chaque image sert aussi à caler nos frames sur celles du jeu.
            crate::horloge::image_du_jeu(temps);
            if (self.tampon.actif() || self.config.historique)
                && let Err(e) = self.lire_image(frame, temps)
            {
                eprintln!("Vidéo : image non lue ({e}).");
            }

            // 3. Selon l'état : préparer, commencer, ou continuer à filmer.
            let mut terminer = false;
            match &mut self.etat {
                Etat::Attente => {
                    let (largeur, hauteur) = (frame.width(), frame.height());
                    let a_jour = self
                        .pret
                        .as_ref()
                        .is_some_and(|p| p.largeur == largeur && p.hauteur == hauteur);
                    if !a_jour && !self.echec_preparation {
                        match self.prendre_encodeur(largeur, hauteur) {
                            Ok(pret) => self.pret = Some(Box::new(pret)),
                            Err(e) => {
                                eprintln!("Vidéo : impossible de préparer l'encodage ({e}).");
                                self.echec_preparation = true;
                            }
                        }
                    }
                }
                Etat::Demande => self.commencer(frame, temps),
                Etat::Filme {
                    encodeur,
                    debut,
                    fin,
                    ..
                } => {
                    if let Err(e) = encodeur.send_frame(frame) {
                        eprintln!("Vidéo : image perdue ({e}).");
                    }
                    terminer = match fin {
                        Some((quand, _)) => Instant::now() >= *quand,
                        // Rien de réussi depuis trop longtemps : on jette.
                        None => debut.elapsed() > Duration::from_secs(self.config.duree_max_s),
                    };
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
            if let Some(pret) = self.pret.take() {
                jeter(*pret);
            }
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
        let mut capture = Capture::new(
            Some(envoi),
            "Dhalsim",
            "Drive Impact, Yoga Flame",
            "b+hp xx DI",
        );
        capture.enregistre = true;
        (capture, reception)
    }

    #[test]
    fn sans_enregistrement_aucune_video() {
        let (mut capture, reception) = capture_de_test();
        capture.enregistre = false;
        capture.debut_essai(10);
        assert!(reception.try_recv().is_err());
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
    fn le_tampon_garde_seulement_les_dernieres_images() {
        // 100 ms gardées ; une image toutes les 16 ms (160 000 unités de 100 ns).
        let mut tampon = Tampon::new(100, usize::MAX);
        for i in 0..20 {
            let pixels = tampon.place();
            tampon.ajouter(Image {
                temps: i * 160_000,
                largeur: 1,
                hauteur: 1,
                pixels,
            });
        }
        // 6 intervalles de 16 ms = 96 ms : les images 13 à 19.
        let images = tampon.prendre();
        assert_eq!(images.len(), 7);
        assert_eq!(images[0].temps, 13 * 160_000);
        assert_eq!(tampon.len(), 0);
    }

    #[test]
    fn le_tampon_respecte_la_limite_de_memoire() {
        let mut tampon = Tampon::new(10_000, 300);
        for i in 0..10 {
            tampon.ajouter(Image {
                temps: i,
                largeur: 10,
                hauteur: 10,
                pixels: vec![0; 100],
            });
        }
        assert_eq!(tampon.len(), 3);
    }

    #[test]
    fn tampon_coupe_si_avant_ms_vaut_zero() {
        assert!(!Tampon::new(0, usize::MAX).actif());
    }

    #[test]
    fn les_lignes_sont_retournees_sans_le_remplissage() {
        // 2 lignes de 2 octets, avec 1 octet de remplissage par ligne (pas = 3).
        let source = [1, 2, 0, 3, 4, 0];
        let mut destination = Vec::new();
        retourner_lignes(&source, 3, 2, 2, &mut destination);
        assert_eq!(destination, vec![3, 4, 1, 2]);
    }

    #[test]
    fn config_par_defaut_valide() {
        let config: Config = toml::from_str(CONFIG_PAR_DEFAUT).unwrap();
        assert!(config.actif);
        assert_eq!(config.processus, "StreetFighter6.exe");
        assert_eq!(config.avant_ms, 700);
        assert!(!config.enregistrer);
    }

    #[test]
    fn ancien_capture_toml_sans_avant_ms() {
        let ancien = CONFIG_PAR_DEFAUT.replace("avant_ms = 700\n", "");
        let config: Config = toml::from_str(&ancien).unwrap();
        assert_eq!(config.avant_ms, 700);
    }
}
