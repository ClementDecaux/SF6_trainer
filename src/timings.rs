//! Les fenêtres de timing de chaque étape, apprises sur tes vrais combos.
//!
//! L'outil ne voit que la manette : il ne sait pas si un coup touche. Il mesure donc
//! l'écart (en frames) entre chaque étape et la précédente. Quand tu vois dans le jeu
//! qu'un combo est vraiment passé, appuie sur Select : les écarts de cet essai sont
//! ajoutés à `timings.toml`. Ensuite, une étape trop tôt ou trop tard est signalée.
//!
//! La frame data publique ne suffit pas pour ces règles : dans un combo avec juggle
//! (après un Drive Impact au mur, par exemple), ce qui compte est l'état de l'adversaire
//! en l'air, pas l'avantage frame. Apprendre sur tes essais marche pour tous les combos.

use serde::{Deserialize, Serialize};

const FICHIER: &str = "timings.toml";

/// Notre horloge et celle du jeu peuvent différer d'une frame : on tolère cet écart.
pub const TOLERANCE: u64 = 1;

/// Les fenêtres ne servent qu'après ce nombre de vrais combos confirmés :
/// avec un seul essai, la fenêtre ferait une seule frame de large.
pub const ESSAIS_MIN: u32 = 3;

const EN_TETE: &str =
    "# Fenêtres de timing apprises sur tes vrais combos (Select après un combo passé).
# min et max : frames entre l'étape d'avant et celle-ci, à partir de la 2e étape.
# Tu peux modifier ces nombres, ou supprimer un combo pour qu'il soit réappris.

";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimingCombo {
    pub nom: String,
    /// Nombre de vrais combos confirmés.
    pub essais: u32,
    pub min: Vec<u64>,
    pub max: Vec<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Timings {
    #[serde(default)]
    pub combo: Vec<TimingCombo>,
}

impl Timings {
    /// Les fenêtres (min, max) d'un combo, si elles ont été apprises pour ce nombre d'étapes
    /// sur au moins `ESSAIS_MIN` vrais combos.
    pub fn fenetres(&self, nom: &str, nombre_etapes: usize) -> Option<Vec<(u64, u64)>> {
        let t = self.combo.iter().find(|t| t.nom == nom)?;
        if t.essais < ESSAIS_MIN || t.min.len() + 1 != nombre_etapes || t.max.len() != t.min.len() {
            return None;
        }
        Some(t.min.iter().copied().zip(t.max.iter().copied()).collect())
    }

    /// Ajoute un vrai combo : `ecarts` = l'écart de chaque étape à partir de la 2e.
    /// Renvoie le nombre d'essais confirmés pour ce combo.
    pub fn apprendre(&mut self, nom: &str, ecarts: &[u64]) -> u32 {
        match self
            .combo
            .iter_mut()
            .find(|t| t.nom == nom && t.min.len() == ecarts.len())
        {
            Some(t) => {
                for (i, &e) in ecarts.iter().enumerate() {
                    t.min[i] = t.min[i].min(e);
                    t.max[i] = t.max[i].max(e);
                }
                t.essais += 1;
                t.essais
            }
            None => {
                // Combo modifié (autre nombre d'étapes) : on repart de zéro.
                self.combo.retain(|t| t.nom != nom);
                self.combo.push(TimingCombo {
                    nom: nom.to_string(),
                    essais: 1,
                    min: ecarts.to_vec(),
                    max: ecarts.to_vec(),
                });
                1
            }
        }
    }

    pub fn enregistrer(&self) -> Result<(), String> {
        let texte = toml::to_string(self).map_err(|e| format!("timings : {e}"))?;
        std::fs::write(FICHIER, format!("{EN_TETE}{texte}"))
            .map_err(|e| format!("impossible d'écrire {FICHIER} : {e}"))
    }
}

/// Charge `timings.toml` (vide s'il n'existe pas encore).
pub fn charger() -> Result<Timings, String> {
    match std::fs::read_to_string(FICHIER) {
        Ok(texte) => toml::from_str(&texte).map_err(|e| format!("erreur dans {FICHIER} : {e}")),
        Err(_) => Ok(Timings::default()),
    }
}

/// Compare un écart à sa fenêtre : `None` si c'est bon, sinon « trop tôt de 2 f »...
pub fn verifier(ecart: u64, (min, max): (u64, u64)) -> Option<String> {
    if ecart + TOLERANCE < min {
        Some(format!("trop tôt de {} f", min - ecart))
    } else if ecart > max + TOLERANCE {
        Some(format!("trop tard de {} f", ecart - max))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apprendre_elargit_la_fenetre() {
        let mut t = Timings::default();
        assert_eq!(t.apprendre("combo", &[12, 30]), 1);
        assert_eq!(t.apprendre("combo", &[14, 28]), 2);
        // Pas encore assez de vrais combos pour s'en servir.
        assert_eq!(t.fenetres("combo", 3), None);
        assert_eq!(t.apprendre("combo", &[13, 29]), 3);
        assert_eq!(t.fenetres("combo", 3), Some(vec![(12, 14), (28, 30)]));
    }

    #[test]
    fn pas_de_fenetre_si_le_combo_a_change() {
        let mut t = Timings::default();
        for _ in 0..3 {
            t.apprendre("combo", &[12, 30]);
        }
        assert_eq!(t.fenetres("combo", 4), None);
        // Réappris avec la nouvelle longueur : l'ancienne entrée est remplacée.
        t.apprendre("combo", &[1, 2, 3]);
        assert_eq!(t.combo.len(), 1);
        assert_eq!(t.combo[0].essais, 1);
    }

    #[test]
    fn verifier_avec_la_tolerance() {
        assert_eq!(verifier(13, (12, 14)), None);
        assert_eq!(verifier(11, (12, 14)), None); // 1 frame de tolérance
        assert_eq!(verifier(10, (12, 14)), Some("trop tôt de 2 f".into()));
        assert_eq!(verifier(17, (12, 14)), Some("trop tard de 3 f".into()));
    }

    #[test]
    fn le_fichier_se_relit() {
        let mut t = Timings::default();
        t.apprendre("Drive Impact, Yoga Flame", &[12, 30]);
        let texte = format!("{EN_TETE}{}", toml::to_string(&t).unwrap());
        let relu: Timings = toml::from_str(&texte).unwrap();
        assert_eq!(relu.combo, t.combo);
    }
}
