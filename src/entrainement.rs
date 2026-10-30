//! Le mode « auto-advance » : une sélection de combos, et le passage automatique au
//! suivant après assez de réussites (au total, ou d'affilée).
//!
//! Réglages dans l'interface (onglet Entraînement), gardés dans `entrainement.toml`.

use crate::combo::Cote;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const FICHIER: &str = "entrainement.toml";

/// Ce qui déclenche le passage au combo suivant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// N réussites d'affilée (un raté remet le compte à zéro).
    #[default]
    Affilee,
    /// N réussites en tout sur ce combo.
    Total,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entrainement {
    /// Auto-advance activé.
    pub actif: bool,
    /// Les combos sélectionnés (par leur nom dans combos.toml).
    pub combos: Vec<String>,
    pub mode: Mode,
    /// Nombre de réussites demandé avant de passer au suivant.
    pub nombre: u32,
    /// Combo suivant au hasard dans la sélection (sinon dans l'ordre).
    pub aleatoire: bool,
    /// De quel côté de l'écran on joue.
    pub cote: Cote,
}

impl Default for Entrainement {
    fn default() -> Self {
        Entrainement {
            actif: false,
            combos: Vec::new(),
            mode: Mode::Affilee,
            nombre: 3,
            aleatoire: false,
            cote: Cote::Auto,
        }
    }
}

/// Charge `entrainement.toml` (réglages par défaut s'il n'existe pas ou est illisible).
pub fn charger() -> Entrainement {
    std::fs::read_to_string(FICHIER)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn enregistrer(e: &Entrainement) {
    match toml::to_string(e) {
        Ok(texte) => {
            let texte = format!(
                "# Réglages de l'entraînement (auto-advance), modifiés depuis l'interface.\n{texte}"
            );
            if let Err(err) = std::fs::write(FICHIER, texte) {
                eprintln!("impossible d'écrire {FICHIER} : {err}");
            }
        }
        Err(err) => eprintln!("impossible d'écrire {FICHIER} : {err}"),
    }
}

impl Entrainement {
    /// Le compte qui compte pour le mode choisi.
    pub fn compte(&self, total: u32, serie: u32) -> u32 {
        match self.mode {
            Mode::Affilee => serie,
            Mode::Total => total,
        }
    }

    /// Assez de réussites pour passer au suivant ?
    pub fn atteint(&self, total: u32, serie: u32) -> bool {
        self.actif && self.compte(total, serie) >= self.nombre.max(1)
    }

    /// Le combo à travailler après `actuel` (index dans `noms`, la liste de combos.toml).
    /// La sélection est parcourue en boucle. Vide : aucun.
    pub fn suivant(&self, actuel: usize, noms: &[String]) -> Option<usize> {
        let selection: Vec<usize> = noms
            .iter()
            .enumerate()
            .filter(|(_, n)| self.combos.contains(n))
            .map(|(i, _)| i)
            .collect();
        if selection.is_empty() {
            return None;
        }
        if self.aleatoire && selection.len() > 1 {
            // Un autre combo de la sélection, au hasard (l'horloge suffit comme hasard).
            let autres: Vec<usize> = selection.into_iter().filter(|&i| i != actuel).collect();
            let hasard = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos() as usize);
            return autres.get(hasard % autres.len()).copied();
        }
        Some(
            selection
                .iter()
                .copied()
                .find(|&i| i > actuel)
                .unwrap_or(selection[0]),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noms() -> Vec<String> {
        ["A", "B", "C", "D"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn le_suivant_tourne_dans_la_selection() {
        let e = Entrainement {
            combos: vec!["B".into(), "D".into()],
            ..Default::default()
        };
        assert_eq!(e.suivant(1, &noms()), Some(3));
        assert_eq!(e.suivant(3, &noms()), Some(1));
        // Combo en cours hors sélection : le prochain de la sélection.
        assert_eq!(e.suivant(2, &noms()), Some(3));
        let vide = Entrainement::default();
        assert_eq!(vide.suivant(0, &noms()), None);
    }

    #[test]
    fn au_hasard_jamais_le_meme() {
        let e = Entrainement {
            combos: vec!["A".into(), "C".into()],
            aleatoire: true,
            ..Default::default()
        };
        for _ in 0..20 {
            assert_eq!(e.suivant(0, &noms()), Some(2));
        }
    }

    #[test]
    fn seuil_selon_le_mode() {
        let mut e = Entrainement {
            actif: true,
            nombre: 3,
            ..Default::default()
        };
        assert!(!e.atteint(5, 2));
        assert!(e.atteint(3, 3));
        e.mode = Mode::Total;
        assert!(e.atteint(5, 0));
        e.actif = false;
        assert!(!e.atteint(5, 5));
    }

    #[test]
    fn les_reglages_s_ecrivent_et_se_relisent() {
        let e = Entrainement {
            actif: true,
            combos: vec!["Drive Rush, Yoga Flame".into()],
            mode: Mode::Total,
            nombre: 5,
            aleatoire: true,
            cote: Cote::Droite,
        };
        let texte = toml::to_string(&e).unwrap();
        assert_eq!(toml::from_str::<Entrainement>(&texte).unwrap(), e);
    }
}
