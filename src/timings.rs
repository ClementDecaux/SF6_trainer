//! Ce qui est appris sur tes vrais combos (Select après un combo passé), dans `timings.toml`.
//!
//! Surtout le nombre de coups qui touchent (lu sur le panneau des dégâts du mode
//! entraînement) : c'est lui qui valide un combo. Les écarts en frames entre étapes
//! (min et max vus) sont gardés pour information ; ils ne servent plus à juger.

use serde::{Deserialize, Serialize};

const FICHIER: &str = "timings.toml";

/// Nombre de vrais combos à confirmer avant de dire que le timing est appris.
pub const ESSAIS_MIN: u32 = 3;

const EN_TETE: &str = "# Appris sur tes vrais combos (Select après un combo passé).
# hits : coups qui touchent quand le combo passe (c'est ce qui valide le combo).
# min et max : frames vues entre l'étape d'avant et celle-ci (pour information).
# Tu peux modifier ces nombres, ou supprimer un combo pour qu'il soit réappris.

";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimingCombo {
    pub nom: String,
    /// Nombre de vrais combos confirmés.
    pub essais: u32,
    pub min: Vec<u64>,
    pub max: Vec<u64>,
    /// Nombre de coups qui touchent quand le combo passe (lu sur le panneau des dégâts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hits: Option<u32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Timings {
    #[serde(default)]
    pub combo: Vec<TimingCombo>,
}

impl Timings {
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
                    hits: None,
                });
                1
            }
        }
    }

    /// Le nombre de coups qui touchent quand ce combo passe, s'il a été appris.
    pub fn hits(&self, nom: &str) -> Option<u32> {
        self.combo.iter().find(|t| t.nom == nom)?.hits
    }

    /// Un vrai combo a touché `hits` fois. On garde le plus petit nombre vu : un combo
    /// réussi ne doit jamais être refusé (un coup en plus, comme un Punish Counter, passe).
    pub fn apprendre_hits(&mut self, nom: &str, hits: u32) {
        if let Some(t) = self.combo.iter_mut().find(|t| t.nom == nom) {
            t.hits = Some(t.hits.map_or(hits, |h| h.min(hits)));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apprendre_elargit_la_fenetre() {
        let mut t = Timings::default();
        assert_eq!(t.apprendre("combo", &[12, 30]), 1);
        assert_eq!(t.apprendre("combo", &[14, 28]), 2);
        assert_eq!(t.apprendre("combo", &[13, 29]), 3);
        assert_eq!(
            (t.combo[0].min.clone(), t.combo[0].max.clone()),
            (vec![12, 28], vec![14, 30])
        );
    }

    #[test]
    fn pas_de_fenetre_si_le_combo_a_change() {
        let mut t = Timings::default();
        for _ in 0..3 {
            t.apprendre("combo", &[12, 30]);
        }
        // Réappris avec la nouvelle longueur : l'ancienne entrée est remplacée.
        t.apprendre("combo", &[1, 2, 3]);
        assert_eq!(t.combo.len(), 1);
        assert_eq!(t.combo[0].essais, 1);
    }

    #[test]
    fn hits_appris() {
        let mut t = Timings::default();
        t.apprendre("combo", &[12]);
        assert_eq!(t.hits("combo"), None);
        t.apprendre_hits("combo", 7);
        t.apprendre_hits("combo", 8);
        assert_eq!(t.hits("combo"), Some(7));
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
