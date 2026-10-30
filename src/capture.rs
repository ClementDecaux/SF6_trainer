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
//!   `clips/`. Les infos du clip (combo, personnage, frame de chaque étape) sont rangées
//!   dans la vidéo elle-même, en JSON, dans une boîte MP4 `uuid` ajoutée à la fin du
//!   fichier : les lecteurs l'ignorent, l'outil la relit. Un seul fichier par combo.
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
# Dossier des photos du jeu (F10). Se change aussi dans les réglages de l'interface.
photos = "photos"
# Combien de temps on filme avant la 1re étape, en millisecondes.
# Ces images sont gardées en mémoire : environ 350 Mo pour 700 ms en 1080p. 0 = rien avant.
avant_ms = 700
# Combien de temps on filme encore après la dernière étape, en millisecondes.
apres_ms = 1500
# Le son du jeu dans les vidéos (seulement celui de SF6, pas le reste de Windows).
son = true
# Qualité de la vidéo, en mégabits par seconde (4 = bon compromis taille/qualité en 1080p,
# environ 2,5 Mo pour un clip de 5 s ; 20 = très bon).
debit_mbps = 4
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
    /// Dossier des photos (F10). Absent des anciens fichiers : « photos ».
    #[serde(default = "photos_par_defaut")]
    pub photos: String,
    /// Absent des anciens fichiers : 700 par défaut.
    #[serde(default = "avant_ms_par_defaut")]
    pub avant_ms: u64,
    pub apres_ms: u64,
    /// Absent des anciens fichiers : `true`.
    #[serde(default = "vrai")]
    pub son: bool,
    pub debit_mbps: u32,
    pub duree_max_s: u64,
}

fn vrai() -> bool {
    true
}

/// Nombre de clips rangés depuis le lancement : l'interface relit ses vidéos quand il change.
pub static CLIPS_RANGES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn photos_par_defaut() -> String {
    "photos".to_string()
}

/// Le dossier des photos du jeu (F10), changeable depuis l'interface pendant que ça tourne.
static DOSSIER_PHOTOS: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn dossier_photos() -> String {
    match DOSSIER_PHOTOS.lock() {
        Ok(d) if !d.is_empty() => d.clone(),
        _ => photos_par_defaut(),
    }
}

/// Change le dossier des photos (et l'écrit dans capture.toml).
pub fn changer_dossier_photos(dossier: &str) {
    if let Ok(mut d) = DOSSIER_PHOTOS.lock() {
        *d = dossier.to_string();
    }
    // Chaîne littérale TOML (entre apostrophes) : les \ des chemins Windows restent tels quels.
    crate::interface::ecrire_cles(FICHIER, &[("photos", format!("'{dossier}'"))]);
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

/// Les infos d'un clip, rangées en JSON dans la vidéo (voir [`ecrire_infos`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfosClip {
    pub personnage: String,
    pub combo: String,
    pub inputs: String,
    /// Date et heure locales, ex. « 2026-10-29 21:03:05 ».
    pub date: String,
    pub fps: u32,
    /// Chaque étape validée, avec sa frame comptée depuis la 1re étape.
    pub etapes: Vec<EtapeClip>,
    /// Heure (horloge de la capture, unités de 100 ns) de la 1re image de la vidéo.
    #[serde(default)]
    pub debut_video: i64,
    /// Délai entre un appui et son affichage dans l'historique du jeu (100 ns).
    #[serde(default)]
    pub latence: i64,
    /// Les lignes de l'historique d'inputs du jeu pendant la vidéo.
    #[serde(default)]
    pub lignes: Vec<crate::ecran::EntreeJeu>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EtapeClip {
    pub notation: String,
    pub frame: u64,
    /// Heure de l'appui (horloge de la capture, 100 ns).
    #[serde(default)]
    pub temps: i64,
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
    /// Délai d'affichage des appuis dans le jeu (écrit dans le `.json`).
    pub latence: i64,
}

impl Capture {
    /// Un autre combo est choisi : les prochaines vidéos portent son nom.
    pub fn changer_combo(&mut self, combo: &str, inputs: &str) {
        self.annuler();
        self.combo = combo.to_string();
        self.inputs = inputs.to_string();
    }

    fn new(envoi: Option<Sender<Ordre>>, personnage: &str, combo: &str, inputs: &str) -> Capture {
        Capture {
            envoi,
            personnage: personnage.to_string(),
            combo: combo.to_string(),
            inputs: inputs.to_string(),
            debut: 0,
            etapes: Vec::new(),
            enregistre: false,
            latence: 0,
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
    pub fn etape(&mut self, notation: &str, frame: u64, temps: i64) {
        self.etapes.push(EtapeClip {
            notation: notation.to_string(),
            frame: frame.saturating_sub(self.debut),
            temps,
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
            debut_video: 0,
            latence: self.latence,
            lignes: Vec::new(),
        };
        self.envoyer(Ordre::Garder(infos));
    }

    pub fn annuler(&mut self) {
        self.etapes.clear();
        self.envoyer(Ordre::Annuler);
    }

    /// Une capture d'écran du jeu (PNG) dans le dossier des photos.
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
    if let Ok(mut d) = DOSSIER_PHOTOS.lock() {
        *d = config.photos.clone();
    }
    if !config.actif {
        println!("Vidéo : coupée dans {FICHIER}.");
        return Capture::new(None, personnage, combo, inputs);
    }
    #[cfg(windows)]
    {
        let (envoi, reception) = std::sync::mpsc::channel();
        let enregistre = config.enregistrer;
        if config.son {
            crate::audio_jeu::demarrer(&config.processus, config.avant_ms);
        }
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

/// Le nom de la vidéo d'un clip, sans extension. Un seul clip par combo : le dernier
/// réussi remplace le précédent.
///
/// Toujours 16 caractères : l'empreinte (FNV-1a 64 bits, en hexadécimal) du personnage et du
/// nom du combo. Une fonction de taille fixe ne peut pas être bijective (il y a plus de combos
/// possibles que de noms), mais on retrouve le combo avec [`combo_du_fichier`], qui calcule
/// l'empreinte des combos connus, et les infos rangées dans la vidéo gardent le nom complet.
pub fn nom_fichier(infos: &InfosClip) -> String {
    empreinte(&infos.personnage, &infos.combo)
}

/// Empreinte de taille fixe (16 caractères hexadécimaux) d'un combo d'un personnage.
/// FNV-1a : stable d'une version de Rust à l'autre, contrairement au hasher standard.
pub fn empreinte(personnage: &str, combo: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for octet in personnage.bytes().chain([0]).chain(combo.bytes()) {
        h ^= u64::from(octet);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Le combo (parmi `noms`) dont la vidéo s'appelle `fichier` (avec ou sans extension).
#[allow(dead_code)]
pub fn combo_du_fichier<'a>(
    fichier: &str,
    personnage: &str,
    noms: &'a [String],
) -> Option<&'a str> {
    let base = fichier.split('.').next().unwrap_or(fichier);
    noms.iter()
        .map(String::as_str)
        .find(|n| empreinte(personnage, n) == base)
}

/// Identifiant de notre boîte `uuid` dans le MP4 (16 octets, lisible dans un éditeur hexa).
const UUID_INFOS: [u8; 16] = *b"sf6trainer-infos";

/// Range les infos du clip dans la vidéo : une boîte MP4 `uuid` ajoutée à la fin du fichier
/// (taille sur 4 octets, « uuid », notre identifiant, puis le JSON). Les lecteurs vidéo
/// sautent les boîtes qu'ils ne connaissent pas : la vidéo se lit toujours partout.
pub fn ecrire_infos(video: &std::path::Path, infos: &InfosClip) -> std::io::Result<()> {
    use std::io::Write;
    let json = serde_json::to_vec(infos).map_err(std::io::Error::other)?;
    let taille = u32::try_from(8 + UUID_INFOS.len() + json.len())
        .map_err(|_| std::io::Error::other("infos du clip trop grandes"))?;
    let mut fichier = std::fs::OpenOptions::new().append(true).open(video)?;
    let mut boite = Vec::with_capacity(taille as usize);
    boite.extend_from_slice(&taille.to_be_bytes());
    boite.extend_from_slice(b"uuid");
    boite.extend_from_slice(&UUID_INFOS);
    boite.extend_from_slice(&json);
    fichier.write_all(&boite)
}

/// Relit les infos rangées dans la vidéo par [`ecrire_infos`] (aucune : `None`).
/// Parcourt les boîtes MP4 du premier niveau sans lire la vidéo elle-même.
pub fn lire_infos(video: &std::path::Path) -> Option<InfosClip> {
    use std::io::{Read, Seek, SeekFrom};
    let mut fichier = std::fs::File::open(video).ok()?;
    let longueur = fichier.metadata().ok()?.len();
    let mut position = 0u64;
    while position + 8 <= longueur {
        fichier.seek(SeekFrom::Start(position)).ok()?;
        let mut entete = [0u8; 8];
        fichier.read_exact(&mut entete).ok()?;
        let mut taille = u64::from(u32::from_be_bytes(entete[..4].try_into().ok()?));
        let mut debut = 8;
        if taille == 1 {
            // Taille sur 64 bits, juste après le type.
            let mut grande = [0u8; 8];
            fichier.read_exact(&mut grande).ok()?;
            taille = u64::from_be_bytes(grande);
            debut = 16;
        } else if taille == 0 {
            // Jusqu'à la fin du fichier.
            taille = longueur - position;
        }
        if taille < debut || position + taille > longueur {
            return None;
        }
        if &entete[4..] == b"uuid" && taille >= debut + 16 {
            let mut uuid = [0u8; 16];
            fichier.read_exact(&mut uuid).ok()?;
            if uuid == UUID_INFOS {
                let mut json = vec![0u8; (taille - debut - 16) as usize];
                fichier.read_exact(&mut json).ok()?;
                return serde_json::from_slice(&json).ok();
            }
        }
        position += taille;
    }
    None
}

/// Supprime les clips déjà gardés pour ce combo. Renvoie le nombre supprimé.
pub fn supprimer_anciens(dossier: &std::path::Path, infos: &InfosClip) -> usize {
    let Ok(entrees) = std::fs::read_dir(dossier) else {
        return 0;
    };
    let mut supprimes = 0;
    for entree in entrees.flatten() {
        let video = entree.path();
        if video.extension().is_none_or(|e| e != "mp4") {
            continue;
        }
        let meme_combo = lire_infos(&video)
            .is_some_and(|i| i.combo == infos.combo && i.personnage == infos.personnage);
        if meme_combo {
            if std::fs::remove_file(&video).is_err() {
                eprintln!(
                    "Vidéo : impossible de supprimer {} (ouverte ?).",
                    video.display()
                );
                continue;
            }
            supprimes += 1;
        }
    }
    supprimes
}

/// Les étapes du traitement d'une image, dans l'ordre de `Image::phases` (la dernière :
/// le total).
pub const PHASES: [&str; 5] = ["ordres", "lecture", "historique", "copie", "total"];

/// Décrit le temps passé dans chaque étape (`phases` en µs), pour le diagnostic.
pub fn decrire_phases(phases: &[u32; 5]) -> String {
    let morceaux: Vec<String> = PHASES
        .iter()
        .zip(phases)
        .map(|(nom, us)| format!("{nom} {:.1}", *us as f64 / 1000.0))
        .collect();
    format!("{} ms", morceaux.join(", "))
}

/// Une image du jeu gardée en mémoire (pixels BGRA, lignes du bas vers le haut).
/// Les pixels sont partagés (`Arc`) : la même image est dans le tampon et chez l'encodeur,
/// sans copie.
#[derive(Clone)]
pub struct Image {
    /// Heure de l'image, en unités de 100 ns (horloge de Windows).
    pub temps: i64,
    pub largeur: u32,
    pub hauteur: u32,
    pub pixels: std::sync::Arc<Vec<u8>>,
    /// Temps passé à traiter cette image dans la capture, en µs (pour trouver les
    /// saccades) : voir `PHASES`.
    pub phases: [u32; 5],
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
            // Plus utilisée par l'encodeur : sa mémoire resservira.
            if let Some(image) = self.images.pop_front()
                && let Ok(pixels) = std::sync::Arc::try_unwrap(image.pixels)
            {
                self.recycler(pixels);
            }
        }
    }

    /// Toutes les images gardées, de la plus vieille à la plus récente. Elles restent dans
    /// le tampon : si l'essai est recommencé pendant le film, le début du nouveau clip est
    /// là (avant, le tampon était vide et le clip commençait à la 1re étape).
    pub fn copier(&self) -> Vec<Image> {
        self.images.iter().cloned().collect()
    }

    pub fn recycler(&mut self, pixels: Vec<u8>) {
        // Quelques-unes suffisent : on en reprend une à chaque image (celles données à
        // l'encodeur reviennent un peu plus tard).
        if self.libres.len() < 6 {
            self.libres.push(pixels);
        }
    }

    /// Note le temps de traitement de la dernière image ajoutée.
    pub fn noter_phases(&mut self, phases: [u32; 5]) {
        if let Some(image) = self.images.back_mut() {
            image.phases = phases;
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
        Config, Image, InfosClip, OCTETS_MAX_TAMPON, Ordre, Tampon, ecrire_infos, nom_fichier,
        retourner_lignes, supprimer_anciens,
    };
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{Receiver, Sender};
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
                            // Pas de limite utile : les images du jeu n'arrivent pas toutes à
                            // 16,7 ms pile. Avec 15 ms, une image arrivée un peu tôt était sautée
                            // (des trous d'une image partout dans les vidéos).
                            MinimumUpdateIntervalSettings::Custom(Duration::from_millis(1)),
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
        /// Échantillons par seconde de la piste son (`None` : vidéo sans son).
        taux: Option<u32>,
    }

    // Un seul `Etat` à la fois, et il ne bouge pas : la taille n'a pas d'importance.
    #[allow(clippy::large_enum_variant)]
    enum Etat {
        Attente,
        /// On a reçu `Demarrer` : on commence à la prochaine image.
        Demande,
        Filme {
            /// L'encodeur, sur son propre fil (voir `lancer_encodage`).
            envoi: Sender<Envoi>,
            fil: std::thread::JoinHandle<bool>,
            chemin: PathBuf,
            debut: Instant,
            /// Heure de la 1re image de la vidéo (horloge de la capture).
            premier: i64,
            /// La vidéo a une piste son, et la prochaine trame de son à lui envoyer.
            son: Option<Option<u64>>,
            /// Combo réussi : quand arrêter, et ce qu'on écrira dans le `.json`.
            fin: Option<(Instant, Box<InfosClip>)>,
            /// Taille des images de la vidéo.
            largeur: u32,
            hauteur: u32,
            /// Pour trouver d'où viennent les saccades : heure de la dernière image, images
            /// qui manquent (trous de plus d'une image), images traitées trop lentement.
            derniere_image: i64,
            manquees: u32,
            lentes: u32,
            /// Pire temps de traitement d'une image pendant le film (voir `PHASES`).
            pire: [u32; 5],
            /// Avant la 1re étape (images du tampon) : manquantes, lentes, pire temps.
            avant: (u32, u32, [u32; 5]),
            /// Les trous : (heure de l'image d'avant, images manquantes), et la durée du
            /// démarrage de la vidéo (encodeur, images d'avant la 1re étape).
            trous: Vec<(i64, u32)>,
            demarrage_ms: u128,
        },
    }

    struct Enregistreur {
        config: Config,
        ordres: Arc<Mutex<Receiver<Ordre>>>,
        etat: Etat,
        pret: Option<Box<Pret>>,
        /// Si la préparation a échoué, on ne réessaie qu'au prochain essai (pas à chaque image).
        echec_preparation: bool,
        /// Encodeur en cours de création sur un fil à part (voir `preparer`).
        preparation: Option<Receiver<Result<Pret, String>>>,
        tampon: Tampon,
        /// Pour donner un nom différent à chaque vidéo en cours.
        numero: u32,
        /// F10 appuyée : la prochaine image est enregistrée en PNG.
        photo: bool,
        /// Lecture de l'historique d'inputs et des dégâts du jeu.
        lecteur: crate::ecran::Lecteur,
        /// Temps passé dans chaque étape pour l'image en cours (voir `PHASES`).
        phases: [u32; 5],
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
                        *fin = Some((quand, Box::new(infos)));
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
                envoi,
                fil,
                chemin,
                fin,
                premier,
                son,
                manquees,
                lentes,
                pire,
                avant,
                trous,
                demarrage_ms,
                ..
            } = etat
            else {
                return;
            };
            // Une piste son sans rien dedans peut bloquer la fin de l'encodage : un peu de
            // silence au minimum.
            if let Some(None) = son
                && let Some(taux) = crate::audio_jeu::taux()
            {
                let silence = vec![0u8; (taux / 10 * 4) as usize];
                let _ = envoi.send(Envoi::Son(silence));
            }
            // Plus rien à envoyer : le fil de l'encodeur termine la vidéo. On ne l'attend pas
            // ici : finir l'encodage prend du temps, et la capture était bloquée pendant ce
            // temps (images perdues au début de l'essai suivant). La fin se fait à part.
            drop(envoi);
            let dossier = self.config.dossier.clone();
            let diagnostic = Diagnostic {
                manquees,
                lentes,
                pire,
                avant,
                trous,
                demarrage_ms,
                encodeurs: self.numero,
            };
            std::thread::spawn(move || {
                if !fil.join().unwrap_or(false) {
                    let _ = std::fs::remove_file(&chemin);
                    return;
                }
                finir_clip(&dossier, &chemin, fin, premier, diagnostic);
            });
        }

        /// Lance la création d'un encodeur sur un fil à part : la créer prend des dizaines
        /// de ms, et faite pendant la capture, elle faisait perdre des images juste avant
        /// l'essai suivant. Le résultat arrive dans `preparation`.
        fn preparer(&mut self, largeur: u32, hauteur: u32) {
            if let Some(ancien) = self.pret.take() {
                jeter_a_part(*ancien);
            }
            self.numero += 1;
            let dossier = self.config.dossier.clone();
            let debit = self.config.debit_mbps;
            let numero = self.numero;
            let taux = self.taux_son();
            let (envoi, reception) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                // Les objets Windows de l'encodeur demandent que COM soit prêt sur ce fil.
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(
                        None,
                        windows::Win32::System::Com::COINIT_MULTITHREADED,
                    );
                }
                let resultat = creer_encodeur(&dossier, debit, numero, largeur, hauteur, taux)
                    .map_err(|e| e.to_string());
                let _ = envoi.send(resultat);
            });
            self.preparation = Some(reception);
        }

        /// Récupère l'encodeur en préparation s'il est prêt (`attendre` : l'attend).
        fn recevoir_preparation(&mut self, attendre: bool) {
            let Some(reception) = &self.preparation else {
                return;
            };
            let resultat = if attendre {
                reception.recv().map_err(|_| ())
            } else {
                match reception.try_recv() {
                    Err(std::sync::mpsc::TryRecvError::Empty) => return,
                    autre => autre.map_err(|_| ()),
                }
            };
            self.preparation = None;
            match resultat {
                Ok(Ok(pret)) => {
                    if let Some(ancien) = self.pret.replace(Box::new(pret)) {
                        jeter_a_part(*ancien);
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("Vidéo : impossible de préparer l'encodage ({e}).");
                    self.echec_preparation = true;
                }
                Err(()) => self.echec_preparation = true,
            }
        }

        /// Le son écouté, s'il est demandé et prêt.
        fn taux_son(&self) -> Option<u32> {
            if self.config.son {
                crate::audio_jeu::taux()
            } else {
                None
            }
        }

        /// Un encodeur à la bonne taille : celui préparé à l'avance, sinon un nouveau.
        fn prendre_encodeur(&mut self, largeur: u32, hauteur: u32) -> Result<Pret, Erreur> {
            // Presque toujours déjà prêt : la préparation commence dès la fin de l'essai.
            self.recevoir_preparation(true);
            match self.pret.take() {
                Some(pret)
                    if pret.largeur == largeur
                        && pret.hauteur == hauteur
                        && pret.taux == self.taux_son() =>
                {
                    Ok(*pret)
                }
                autre => {
                    if let Some(pret) = autre {
                        jeter_a_part(*pret);
                    }
                    self.numero += 1;
                    creer_encodeur(
                        &self.config.dossier,
                        self.config.debit_mbps,
                        self.numero,
                        largeur,
                        hauteur,
                        self.taux_son(),
                    )
                }
            }
        }

        /// Lit l'historique d'inputs du jeu sur l'image. Si l'image entière est utile (tampon
        /// des images d'avant la 1re étape, ou vidéo en cours), elle est copiée en mémoire et
        /// renvoyée ; sinon, seule la partie lue est copiée depuis la carte graphique.
        fn lire_image(&mut self, frame: &mut Frame, temps: i64) -> Result<Option<Image>, Erreur> {
            let (largeur, hauteur) = (frame.width(), frame.height());
            let garder = self.tampon.actif() || !matches!(self.etat, Etat::Attente);
            if !garder {
                if !self.config.historique {
                    return Ok(None);
                }
                let (x0, y0, x1, y1) = crate::ecran::Vue::zone_lue(largeur, hauteur);
                let mut morceau = frame.buffer_crop(x0, y0, x1, y1)?;
                let pas = morceau.row_pitch() as usize;
                let brut = morceau.as_raw_buffer();
                let vue = crate::ecran::Vue::zone(
                    brut,
                    (largeur, hauteur),
                    ((x1 - x0) as usize, (y1 - y0) as usize),
                    pas,
                );
                crate::ecran::lire_image(&mut self.lecteur, &vue, temps);
                return Ok(None);
            }
            let chrono = Instant::now();
            let mut image = frame.buffer()?;
            let pas = image.row_pitch() as usize;
            let brut = image.as_raw_buffer();
            self.phases[1] = chrono.elapsed().as_micros() as u32;
            let chrono = Instant::now();
            if self.config.historique {
                let vue = crate::ecran::Vue::jeu(brut, largeur as usize, hauteur as usize, pas);
                crate::ecran::lire_image(&mut self.lecteur, &vue, temps);
            }
            self.phases[2] = chrono.elapsed().as_micros() as u32;
            let chrono = Instant::now();
            let mut pixels = self.tampon.place();
            retourner_lignes(
                brut,
                pas,
                largeur as usize * 4,
                hauteur as usize,
                &mut pixels,
            );
            self.phases[3] = chrono.elapsed().as_micros() as u32;
            Ok(Some(Image {
                temps,
                largeur,
                hauteur,
                pixels: std::sync::Arc::new(pixels),
                phases: [0; 5],
            }))
        }

        /// Démarre la vidéo avec les images du tampon (l'image actuelle comprise).
        fn commencer(&mut self, frame: &Frame, temps: i64) {
            let chrono = Instant::now();
            let (largeur, hauteur) = (frame.width(), frame.height());
            let Pret {
                encodeur,
                chemin,
                taux,
                ..
            } = match self.prendre_encodeur(largeur, hauteur) {
                Ok(pret) => pret,
                Err(e) => {
                    eprintln!("Vidéo : impossible de démarrer l'encodage ({e}).");
                    self.etat = Etat::Attente;
                    return;
                }
            };
            let (envoi, fil) = lancer_encodage(encodeur);
            let mut manquees = 0;
            let mut trous = Vec::new();
            let mut premier = None;
            let mut precedente = None;
            let mut avant = (0, 0, [0; 5]);
            for image in self.tampon.copier() {
                if image.largeur != largeur || image.hauteur != hauteur {
                    continue;
                }
                if let Some(p) = precedente {
                    let n = images_manquees(image.temps - p);
                    if n > 0 {
                        manquees += n;
                        avant.0 += n;
                        trous.push((p, n));
                    }
                }
                if image.phases[4] > 14_000 {
                    avant.1 += 1;
                }
                if image.phases[4] > avant.2[4] {
                    avant.2 = image.phases;
                }
                premier.get_or_insert(image.temps);
                precedente = Some(image.temps);
                let _ = envoi.send(Envoi::Image(image));
            }
            let premier = premier.unwrap_or(temps);
            // Le son commence à l'heure de la 1re image : il est calé dessus.
            let mut son = taux.map(|_| crate::audio_jeu::numero(premier));
            envoyer_son(&envoi, &mut son, premier, temps);
            self.etat = Etat::Filme {
                envoi,
                fil,
                chemin,
                debut: Instant::now(),
                premier,
                son,
                fin: None,
                largeur,
                hauteur,
                derniere_image: temps,
                manquees,
                lentes: 0,
                pire: [0; 5],
                avant,
                trous,
                demarrage_ms: chrono.elapsed().as_millis(),
            };
        }
    }

    /// Ce qu'on a mesuré pendant le film, pour trouver d'où viennent les saccades.
    struct Diagnostic {
        manquees: u32,
        lentes: u32,
        pire: [u32; 5],
        avant: (u32, u32, [u32; 5]),
        trous: Vec<(i64, u32)>,
        demarrage_ms: u128,
        encodeurs: u32,
    }

    /// La vidéo est encodée : gardée et rangée si le combo a été réussi, jetée sinon.
    fn finir_clip(
        dossier: &str,
        chemin: &Path,
        fin: Option<(Instant, Box<InfosClip>)>,
        premier: i64,
        d: Diagnostic,
    ) {
        let Some((_, mut infos)) = fin else {
            let _ = std::fs::remove_file(chemin);
            return;
        };
        // Pour le lecteur : où commence la vidéo, et l'historique du jeu pendant.
        infos.debut_video = premier;
        infos.lignes = crate::ecran::entrees_depuis(premier);
        ranger(dossier, chemin, &infos);
        if d.manquees > 0 || d.lentes > 0 {
            let liste: Vec<String> = d
                .trous
                .iter()
                .take(40)
                .map(|&(t, n)| format!("à {:.2} s : {n}", (t - premier) as f64 / 10_000_000.0))
                .collect();
            println!(
                "      Vidéo : {} image(s) manquante(s) (saccades), {} image(s) traitée(s) en \
                 plus de 14 ms (pire : {}), démarrage en {} ms. Avant la 1re étape : {} \
                 manquante(s), {} lente(s) (pire : {}). Encodeurs créés : {}. Trous : {}",
                d.manquees,
                d.lentes,
                super::decrire_phases(&d.pire),
                d.demarrage_ms,
                d.avant.0,
                d.avant.1,
                super::decrire_phases(&d.avant.2),
                d.encodeurs,
                liste.join(", ")
            );
        }
    }

    /// Renomme la vidéo et y range ses infos.
    fn ranger(dossier: &str, en_cours: &Path, infos: &InfosClip) {
        // Un seul exemplaire par combo : l'ancien clip laisse la place au nouveau.
        supprimer_anciens(Path::new(dossier), infos);
        let video = Path::new(dossier)
            .join(nom_fichier(infos))
            .with_extension("mp4");
        let _ = std::fs::remove_file(&video);
        if let Err(e) = std::fs::rename(en_cours, &video) {
            eprintln!("Vidéo : impossible de ranger le clip ({e}).");
            return;
        }
        if let Err(e) = ecrire_infos(&video, infos) {
            eprintln!("Vidéo : impossible d'y ranger les infos du clip ({e}).");
        }
        println!("      🎬 clip enregistré : {}", video.display());
        super::CLIPS_RANGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Ce qu'on donne au fil de l'encodeur.
    enum Envoi {
        Image(Image),
        Son(Vec<u8>),
    }

    /// L'encodeur tourne sur son propre fil. Lui donner une image la copie (8 Mo en
    /// 1080p) : fait pendant la capture, ça la ralentissait et des images du jeu étaient
    /// perdues (saccades). Ici, la capture ne fait que lui passer l'image, sans copie. La
    /// mémoire de l'image resservira quand le tampon l'aura lâchée. Le fil se termine (et la
    /// vidéo avec) quand `Envoi` n'a plus d'expéditeur ; il dit si tout s'est bien passé.
    fn lancer_encodage(
        mut encodeur: VideoEncoder,
    ) -> (Sender<Envoi>, std::thread::JoinHandle<bool>) {
        let (envoi, reception) = std::sync::mpsc::channel::<Envoi>();
        let fil = std::thread::spawn(move || {
            for envoye in reception {
                match envoye {
                    Envoi::Image(image) => {
                        if let Err(e) = encodeur.send_frame_buffer(&image.pixels, image.temps) {
                            eprintln!("Vidéo : image perdue ({e}).");
                        }
                    }
                    Envoi::Son(octets) => {
                        if let Err(e) = encodeur.send_audio_buffer(&octets, 0) {
                            eprintln!("Vidéo : son perdu ({e}).");
                        }
                    }
                }
            }
            match encodeur.finish() {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("Vidéo : erreur à la fin de l'encodage ({e}).");
                    false
                }
            }
        });
        (envoi, fil)
    }

    /// Envoie à l'encodeur le son reçu depuis la dernière fois, jusqu'à l'heure `temps`.
    fn envoyer_son(envoi: &Sender<Envoi>, son: &mut Option<Option<u64>>, premier: i64, temps: i64) {
        let Some(curseur) = son else {
            return;
        };
        let Some(depuis) = curseur.or_else(|| crate::audio_jeu::numero(premier)) else {
            return;
        };
        let (octets, suivant) = crate::audio_jeu::lire(depuis, temps);
        *curseur = Some(suivant);
        if !octets.is_empty() {
            let _ = envoi.send(Envoi::Son(octets));
        }
    }

    /// Images qui manquent entre deux images reçues à `ecart` d'intervalle (100 ns) à 60 fps.
    fn images_manquees(ecart: i64) -> u32 {
        const IMAGE: i64 = 166_667;
        ((ecart + IMAGE / 2) / IMAGE - 1).max(0) as u32
    }

    fn creer_encodeur(
        dossier: &str,
        debit_mbps: u32,
        numero: u32,
        largeur: u32,
        hauteur: u32,
        taux: Option<u32>,
    ) -> Result<Pret, Erreur> {
        std::fs::create_dir_all(dossier)?;
        let chemin = Path::new(dossier).join(format!("en_cours_{numero}.mp4"));
        let video = VideoSettingsBuilder::new(largeur, hauteur)
            .sub_type(VideoSettingsSubType::H264)
            .frame_rate(60)
            .bitrate(debit_mbps.saturating_mul(1_000_000));
        let son = match taux {
            Some(taux) => AudioSettingsBuilder::default()
                .sample_rate(taux)
                .channel_count(2)
                .bit_per_sample(16),
            None => AudioSettingsBuilder::default().disabled(true),
        };
        let encodeur = VideoEncoder::new(video, son, ContainerSettingsBuilder::default(), &chemin)?;
        Ok(Pret {
            encodeur,
            chemin,
            largeur,
            hauteur,
            taux,
        })
    }

    /// Comme `jeter`, mais sans bloquer la capture.
    fn jeter_a_part(pret: Pret) {
        std::thread::spawn(move || jeter(pret));
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
                preparation: None,
                tampon,
                numero: 0,
                photo: false,
                lecteur: crate::ecran::Lecteur::default(),
                phases: [0; 5],
            })
        }

        /// Appelé à chaque nouvelle image du jeu (60 fois par seconde).
        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            _controle: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let arrivee = Instant::now();
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
                let dossier = super::dossier_photos();
                let chemin = Path::new(&dossier).join(nom);
                let _ = std::fs::create_dir_all(&dossier);
                match frame.save_as_image(&chemin, ImageFormat::Png) {
                    Ok(()) => println!("      📷 photo du jeu : {}", chemin.display()),
                    Err(e) => eprintln!("Vidéo : photo impossible ({e})."),
                }
            }

            // 2. L'image est lue (historique du jeu) et va dans le tampon des dernières images.
            let temps = frame.timestamp()?.Duration;
            self.phases = [0; 5];
            self.phases[0] = arrivee.elapsed().as_micros() as u32;
            // L'heure de chaque image sert aussi à caler nos frames sur celles du jeu.
            crate::horloge::image_du_jeu(temps);
            let mut image = match self.lire_image(frame, temps) {
                Ok(image) => image,
                Err(e) => {
                    eprintln!("Vidéo : image non lue ({e}).");
                    None
                }
            };
            // L'image va toujours dans le tampon des dernières images (même pendant le film :
            // la mémoire est partagée avec l'encodeur, pas copiée).
            if let Some(image) = &image {
                self.tampon.ajouter(image.clone());
            }

            // 3. Selon l'état : préparer, commencer, ou continuer à filmer.
            let mut terminer = false;
            match &mut self.etat {
                Etat::Attente => {
                    self.recevoir_preparation(false);
                    let (largeur, hauteur) = (frame.width(), frame.height());
                    let a_jour = self.pret.as_ref().is_some_and(|p| {
                        p.largeur == largeur && p.hauteur == hauteur && p.taux == self.taux_son()
                    });
                    if !a_jour && !self.echec_preparation && self.preparation.is_none() {
                        self.preparer(largeur, hauteur);
                    }
                }
                Etat::Demande => {
                    self.commencer(frame, temps);
                    // L'encodeur du prochain essai se prépare déjà : recommencer pendant le
                    // film ne fait plus attendre sa création (plus de 200 ms).
                    if matches!(self.etat, Etat::Filme { .. }) {
                        self.preparer(frame.width(), frame.height());
                    }
                }
                Etat::Filme {
                    envoi,
                    debut,
                    fin,
                    premier,
                    son,
                    largeur,
                    hauteur,
                    derniere_image,
                    manquees,
                    trous,
                    ..
                } => {
                    let n = images_manquees(temps - *derniere_image);
                    if n > 0 {
                        *manquees += n;
                        trous.push((*derniere_image, n));
                    }
                    *derniere_image = temps;
                    // L'image copiée en mémoire, pas la texture de la capture : Windows réutilise
                    // cette texture pour l'image suivante avant que l'encodeur l'ait lue, et la
                    // vidéo montrait alors des images répétées (saccades). Pas copiée (erreur)
                    // ou taille changée : image sautée.
                    if let Some(i) =
                        image.take_if(|i| i.largeur == *largeur && i.hauteur == *hauteur)
                    {
                        let _ = envoi.send(Envoi::Image(i));
                    }
                    envoyer_son(envoi, son, *premier, temps);
                    terminer = match fin {
                        Some((quand, _)) => Instant::now() >= *quand,
                        // Rien de réussi depuis trop longtemps : on jette.
                        None => debut.elapsed() > Duration::from_secs(self.config.duree_max_s),
                    };
                }
            }
            drop(image);
            self.phases[4] = arrivee.elapsed().as_micros() as u32;
            let phases = self.phases;
            self.tampon.noter_phases(phases);
            if let Etat::Filme { lentes, pire, .. } = &mut self.etat {
                if phases[4] > 14_000 {
                    *lentes += 1;
                }
                if phases[4] > pire[4] {
                    *pire = phases;
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
            self.recevoir_preparation(true);
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
        capture.etape("4HP", 1000, 0);
        capture.etape("DI", 1012, 0);
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
                    frame: 0,
                    temps: 0,
                },
                EtapeClip {
                    notation: "DI".into(),
                    frame: 12,
                    temps: 0,
                },
            ]
        );
    }

    #[test]
    fn nouvel_essai_efface_les_etapes() {
        let (mut capture, _reception) = capture_de_test();
        capture.debut_essai(10);
        capture.etape("4HP", 10, 0);
        capture.debut_essai(500);
        capture.etape("4HP", 500, 0);
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
            debut_video: 0,
            latence: 0,
            lignes: vec![],
        };
        let nom = nom_fichier(&infos);
        assert_eq!(nom.len(), 16);
        assert!(nom.chars().all(|c| c.is_ascii_hexdigit()));
        // Toujours la même empreinte, quelle que soit la longueur du nom.
        assert_eq!(nom, nom_fichier(&infos));
        assert_eq!(super::empreinte("Dhalsim", "a").len(), 16);
        assert_eq!(
            super::empreinte("Dhalsim", &"très long combo ".repeat(20)).len(),
            16
        );
        // Et on retrouve le combo depuis le nom de la vidéo.
        let noms = vec!["2MK, Yoga Fire".to_string(), infos.combo.clone()];
        assert_eq!(
            super::combo_du_fichier(&format!("{nom}.mp4"), "Dhalsim", &noms),
            Some(infos.combo.as_str())
        );
        assert_eq!(
            super::combo_du_fichier("0123456789abcdef", "Dhalsim", &noms),
            None
        );
    }

    /// Un petit MP4 : une boîte `ftyp`, puis une `mdat` avec une taille sur 64 bits.
    fn mp4_factice() -> Vec<u8> {
        let mut octets = Vec::new();
        octets.extend_from_slice(&16u32.to_be_bytes());
        octets.extend_from_slice(b"ftypisom\0\0\0\0");
        octets.extend_from_slice(&1u32.to_be_bytes());
        octets.extend_from_slice(b"mdat");
        octets.extend_from_slice(&20u64.to_be_bytes());
        octets.extend_from_slice(b"abcd");
        octets
    }

    #[test]
    fn les_infos_sont_rangees_dans_la_video() {
        use super::{ecrire_infos, lire_infos};
        let video = std::env::temp_dir().join(format!("sf6_infos_{}.mp4", std::process::id()));
        std::fs::write(&video, mp4_factice()).unwrap();
        assert!(lire_infos(&video).is_none());
        let infos = InfosClip {
            personnage: "Dhalsim".into(),
            combo: "Drive Rush, Yoga Flame".into(),
            inputs: "5LP xx DR, 5LP".into(),
            date: "2026-10-30 18:00:00".into(),
            fps: 60,
            etapes: vec![],
            debut_video: 12,
            latence: 3,
            lignes: vec![],
        };
        ecrire_infos(&video, &infos).unwrap();
        let relu = lire_infos(&video).unwrap();
        assert_eq!(
            (relu.combo, relu.inputs, relu.debut_video),
            (infos.combo, infos.inputs, 12)
        );
        // La vidéo d'origine est intacte, la boîte est juste ajoutée à la fin.
        let octets = std::fs::read(&video).unwrap();
        assert_eq!(&octets[..mp4_factice().len()], &mp4_factice()[..]);
        let _ = std::fs::remove_file(&video);
    }

    #[test]
    fn les_anciens_clips_du_combo_sont_supprimes() {
        let dossier = std::env::temp_dir().join(format!("sf6_clips_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dossier);
        std::fs::create_dir_all(&dossier).unwrap();
        let infos = |combo: &str| InfosClip {
            personnage: "Dhalsim".into(),
            combo: combo.into(),
            inputs: String::new(),
            date: "2026-10-29 21:03:05".into(),
            fps: 60,
            etapes: vec![],
            debut_video: 0,
            latence: 0,
            lignes: vec![],
        };
        for (nom, combo) in [("a", "Un"), ("b", "Un"), ("c", "Deux")] {
            let video = dossier.join(format!("{nom}.mp4"));
            std::fs::write(&video, mp4_factice()).unwrap();
            ecrire_infos(&video, &infos(combo)).unwrap();
        }
        assert_eq!(supprimer_anciens(&dossier, &infos("Un")), 2);
        assert!(!dossier.join("a.mp4").exists());
        assert!(!dossier.join("b.mp4").exists());
        assert!(dossier.join("c.mp4").exists());
        let _ = std::fs::remove_dir_all(&dossier);
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
                pixels: std::sync::Arc::new(pixels),
                phases: [0; 5],
            });
        }
        // 6 intervalles de 16 ms = 96 ms : les images 13 à 19.
        let images = tampon.copier();
        assert_eq!(images.len(), 7);
        assert_eq!(images[0].temps, 13 * 160_000);
        assert_eq!(tampon.len(), 7);
    }

    #[test]
    fn le_tampon_respecte_la_limite_de_memoire() {
        let mut tampon = Tampon::new(10_000, 300);
        for i in 0..10 {
            tampon.ajouter(Image {
                temps: i,
                largeur: 10,
                hauteur: 10,
                pixels: std::sync::Arc::new(vec![0; 100]),
                phases: [0; 5],
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
