//! Correspondance entre les boutons physiques et les actions de SF6.
//!
//! Le programme voit des boutons façon manette Xbox (`West`, `South`...).
//! Le fichier `controles.toml` dit ce que chacun fait dans SF6.
//! S'il n'existe pas, il est créé avec la config par défaut de SF6 (Classic, Xbox).

use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;

const FICHIER: &str = "controles.toml";

const CONFIG_PAR_DEFAUT: &str = r#"# Bouton lu par le programme = action dans SF6 (contrôles Classic).
# Lance `cargo run`, appuie sur un bouton, et regarde le nom affiché pour savoir lequel modifier.
# Actions possibles : LP, MP, HP, LK, MK, HK, DP (Drive Parry), DI (Drive Impact), Throw
West = "LP"
North = "MP"
RightTrigger = "HP"
South = "LK"
East = "MK"
RightTrigger2 = "HK"
LeftTrigger = "DP"
LeftTrigger2 = "DI"
# LeftThumb = "Throw"
# RightThumb = "?"
"#;

/// Une action de SF6 déclenchée par un bouton.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
pub enum Action {
    LP,
    MP,
    HP,
    LK,
    MK,
    HK,
    /// Drive Parry
    DP,
    /// Drive Impact
    DI,
    Throw,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Le nom affiché est le même que le nom de la variante (LP, MK...).
        write!(f, "{self:?}")
    }
}

/// Clé = nom du bouton physique (ex. "West"), valeur = action SF6.
pub type Controles = HashMap<String, Action>;

/// Charge `controles.toml`, ou le crée avec la config par défaut s'il n'existe pas.
pub fn charger() -> Result<Controles, String> {
    let texte = match std::fs::read_to_string(FICHIER) {
        Ok(texte) => texte,
        Err(_) => {
            std::fs::write(FICHIER, CONFIG_PAR_DEFAUT)
                .map_err(|e| format!("impossible de créer {FICHIER} : {e}"))?;
            println!("{FICHIER} créé avec la config par défaut.");
            CONFIG_PAR_DEFAUT.to_string()
        }
    };
    toml::from_str(&texte).map_err(|e| format!("erreur dans {FICHIER} : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_config_par_defaut_se_lit() {
        let controles: Controles = toml::from_str(CONFIG_PAR_DEFAUT).unwrap();
        assert_eq!(controles["West"], Action::LP);
        assert_eq!(controles["RightTrigger2"], Action::HK);
        assert_eq!(controles.len(), 8);
    }

    #[test]
    fn une_action_inconnue_est_refusee() {
        let resultat: Result<Controles, _> = toml::from_str(r#"West = "Hadoken""#);
        assert!(resultat.is_err());
    }
}
