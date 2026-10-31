//! Partage des clips en ligne : envoi d'un clip au serveur (dossier `serveur/`), qui le
//! range dans le stockage et l'ajoute au catalogue que tout le monde peut voir.
//!
//! Réglages dans `nuage.toml` : l'adresse du serveur et le pseudo. Les clips partagés sont
//! notés dans `partages.json` (dans le dossier des clips), avec le jeton qui permet de les
//! retirer.

use crate::capture::InfosClip;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FICHIER: &str = "nuage.toml";

const CONFIG_PAR_DEFAUT: &str = r#"# Partage des clips en ligne.
# Adresse du serveur de partage (vide : partage désactivé).
serveur = ''
# Le nom affiché à côté de tes clips (2 à 24 caractères).
pseudo = ''
"#;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub serveur: String,
    #[serde(default)]
    pub pseudo: String,
}

/// Lit `nuage.toml` (le crée s'il n'existe pas).
pub fn charger() -> Config {
    let texte = match std::fs::read_to_string(FICHIER) {
        Ok(t) => t,
        Err(_) => {
            let _ = std::fs::write(FICHIER, CONFIG_PAR_DEFAUT);
            CONFIG_PAR_DEFAUT.to_string()
        }
    };
    toml::from_str(&texte).unwrap_or_default()
}

/// Un clip partagé : de quoi le reconnaître et le retirer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Partage {
    pub id: String,
    pub jeton: String,
    /// Taille du fichier envoyé : si le clip a été refilmé depuis, ce n'est plus le même.
    pub taille: u64,
}

/// Où en est le partage d'un clip.
#[derive(Debug, Clone, PartialEq)]
pub enum Etat {
    Envoi,
    Partage(Partage),
    Retrait,
    Erreur(String),
}

/// L'état des envois en cours ou finis, par vidéo.
static ETATS: Mutex<Option<HashMap<PathBuf, Etat>>> = Mutex::new(None);

/// L'état du partage de `video` : envoi en cours, déjà partagé (noté dans `partages.json`)...
pub fn etat(video: &Path) -> Option<Etat> {
    if let Ok(etats) = ETATS.lock()
        && let Some(e) = etats.as_ref().and_then(|e| e.get(video))
    {
        return Some(e.clone());
    }
    let partage = lire_partages(video).remove(&nom(video))?;
    let taille = std::fs::metadata(video).map(|m| m.len()).unwrap_or(0);
    (partage.taille == taille).then_some(Etat::Partage(partage))
}

fn noter(video: &Path, etat: Etat) {
    if let Ok(mut etats) = ETATS.lock() {
        etats
            .get_or_insert_with(HashMap::new)
            .insert(video.to_path_buf(), etat);
    }
}

fn nom(video: &Path) -> String {
    video
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn fichier_partages(video: &Path) -> PathBuf {
    video
        .parent()
        .unwrap_or(Path::new("."))
        .join("partages.json")
}

fn lire_partages(video: &Path) -> HashMap<String, Partage> {
    std::fs::read_to_string(fichier_partages(video))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn ecrire_partage(video: &Path, partage: Option<Partage>) {
    let mut partages = lire_partages(video);
    match partage {
        Some(p) => partages.insert(nom(video), p),
        None => partages.remove(&nom(video)),
    };
    if let Ok(texte) = serde_json::to_string_pretty(&partages) {
        let _ = std::fs::write(fichier_partages(video), texte);
    }
}

/// Envoie le clip au serveur, sur un fil à part (l'interface suit avec `etat`).
pub fn partager(video: PathBuf, infos: InfosClip, config: Config) {
    noter(&video, Etat::Envoi);
    std::thread::spawn(move || {
        let etat = match envoyer(&video, &infos, &config) {
            Ok(partage) => {
                ecrire_partage(&video, Some(partage.clone()));
                Etat::Partage(partage)
            }
            Err(e) => Etat::Erreur(e),
        };
        noter(&video, etat);
    });
}

/// Retire du serveur un clip partagé.
pub fn retirer(video: PathBuf, partage: Partage, config: Config) {
    noter(&video, Etat::Retrait);
    std::thread::spawn(move || {
        let reponse = agent()
            .delete(&format!("{}/clips/{}", base(&config), partage.id))
            .header("X-Jeton", &partage.jeton)
            .call();
        let etat = match reponse.map_err(|e| e.to_string()).and_then(verifier) {
            // Déjà retiré (par l'admin par exemple) : on l'oublie aussi.
            Ok(_) => None,
            Err(e) if e.contains("clip inconnu") => None,
            Err(e) => Some(Etat::Erreur(e)),
        };
        if etat.is_none() {
            ecrire_partage(&video, None);
        }
        if let Ok(mut etats) = ETATS.lock()
            && let Some(etats) = etats.as_mut()
        {
            match etat {
                Some(e) => etats.insert(video, e),
                None => etats.remove(&video),
            };
        }
    });
}

#[derive(Deserialize)]
struct Autorisation {
    id: String,
    jeton: String,
    envoi: Envoi,
}

#[derive(Deserialize)]
struct Envoi {
    adresse: String,
    entetes: HashMap<String, String>,
}

fn base(config: &Config) -> String {
    config.serveur.trim().trim_end_matches('/').to_string()
}

/// Les erreurs du serveur (4xx, 5xx) sont lues comme des réponses : il dit pourquoi.
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .build()
        .into()
}

/// Le corps d'une réponse réussie, ou le message d'erreur du serveur.
fn verifier(mut reponse: ureq::http::Response<ureq::Body>) -> Result<String, String> {
    let statut = reponse.status();
    let corps = reponse
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    if statut.is_success() {
        return Ok(corps);
    }
    let message = serde_json::from_str::<serde_json::Value>(&corps)
        .ok()
        .and_then(|v| v["erreur"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("erreur {statut}"));
    Err(message)
}

fn envoyer(video: &Path, infos: &InfosClip, config: &Config) -> Result<Partage, String> {
    if config.serveur.trim().is_empty() {
        return Err(format!("pas de serveur de partage (voir {FICHIER})"));
    }
    let octets = std::fs::read(video).map_err(|e| format!("clip illisible ({e})"))?;
    let agent = agent();

    // 1. Autorisation : le serveur vérifie les limites et donne l'adresse d'envoi.
    let demande = serde_json::json!({
        "pseudo": config.pseudo.trim(),
        "personnage": infos.personnage,
        "combo": infos.combo,
        "taille": octets.len(),
    });
    let reponse = agent
        .post(&format!("{}/clips", base(config)))
        .send_json(&demande)
        .map_err(|e| format!("serveur injoignable ({e})"))?;
    let autorisation: Autorisation =
        serde_json::from_str(&verifier(reponse)?).map_err(|e| e.to_string())?;

    // 2. La vidéo, directement dans le stockage.
    let mut envoi = agent.put(&autorisation.envoi.adresse);
    for (nom, valeur) in &autorisation.envoi.entetes {
        // Content-Length est mis par ureq lui-même (taille du corps).
        if !nom.eq_ignore_ascii_case("content-length") {
            envoi = envoi.header(nom, valeur);
        }
    }
    let reponse = envoi
        .send(&octets[..])
        .map_err(|e| format!("envoi interrompu ({e})"))?;
    if !reponse.status().is_success() {
        return Err(format!("stockage : erreur {}", reponse.status()));
    }

    // 3. C'est envoyé : le serveur vérifie et publie.
    let reponse = agent
        .post(&format!("{}/clips/{}/fini", base(config), autorisation.id))
        .send_empty()
        .map_err(|e| format!("serveur injoignable ({e})"))?;
    verifier(reponse)?;
    Ok(Partage {
        id: autorisation.id,
        jeton: autorisation.jeton,
        taille: octets.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_par_defaut_valide() {
        let c: Config = toml::from_str(CONFIG_PAR_DEFAUT).unwrap();
        assert!(c.serveur.is_empty() && c.pseudo.is_empty());
    }

    #[test]
    fn un_partage_est_oublie_si_le_clip_a_ete_refilme() {
        let dossier = std::env::temp_dir().join(format!("sf6_nuage_{}", std::process::id()));
        std::fs::create_dir_all(&dossier).unwrap();
        let video = dossier.join("abc.mp4");
        std::fs::write(&video, [0u8; 10]).unwrap();
        let partage = Partage {
            id: "1".into(),
            jeton: "j".into(),
            taille: 10,
        };
        ecrire_partage(&video, Some(partage.clone()));
        assert_eq!(etat(&video), Some(Etat::Partage(partage)));
        std::fs::write(&video, [0u8; 12]).unwrap();
        assert_eq!(etat(&video), None);
        let _ = std::fs::remove_dir_all(&dossier);
    }
}
