//! Affichage de l'historique des inputs dans le terminal, façon « input display » de SF6.
//!
//! Une ligne = un état (direction + boutons tenus) et le nombre de frames qu'il a duré.
//! Exemple :
//!
//!    3  → 6
//!    3  ↓ 2
//!    1  ↙ 1
//!    2  ← 4
//!    2  • 5
//!    3  • 5  HP          ► 63214HP
//!
//! La ligne s'affiche quand l'état se termine (avant, on ne connaît pas sa durée).

use crate::combo::{Liaison, Trial};
use crate::controles::Action;
use colored::{ColoredString, Colorize};

/// Au-delà, on affiche « 99+ » comme le jeu.
const MAX_FRAMES: u64 = 99;

/// Ce qui est tenu à un instant donné.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Etat {
    pub direction: u8,
    /// Boutons tenus, triés (LP, MP, HP, LK...).
    pub boutons: Vec<Action>,
}

/// Construit la ligne d'un état qui vient de se terminer.
/// `nouveaux` = boutons appuyés au début de cet état (affichés en gras),
/// `coups` = coups reconnus à ce moment-là (ex. "236HP").
pub fn ligne(etat: &Etat, frames: u64, nouveaux: &[Action], coups: &[String]) -> String {
    let duree = if frames > MAX_FRAMES {
        "99+".to_string()
    } else {
        format!("{frames:>3}")
    };

    let mut texte = format!("{}  {}", duree.cyan(), direction(etat.direction));

    for &b in &etat.boutons {
        let nom = bouton(b);
        // Bouton appuyé à cette frame : en gras. Bouton simplement tenu : atténué.
        let nom = if nouveaux.contains(&b) {
            nom.bold()
        } else {
            nom.dimmed()
        };
        texte.push_str(&format!("  {nom}"));
    }

    for coup in coups {
        texte.push_str(&format!(
            "    {}",
            format!("► {coup}").bright_yellow().bold()
        ));
    }
    texte
}

/// La direction : flèche + chiffre numpad. Le neutre est atténué.
fn direction(numpad: u8) -> ColoredString {
    let fleche = match numpad {
        1 => "↙",
        2 => "↓",
        3 => "↘",
        4 => "←",
        6 => "→",
        7 => "↖",
        8 => "↑",
        9 => "↗",
        _ => "•",
    };
    let texte = format!("{fleche} {numpad}");
    if numpad == 5 {
        texte.dimmed()
    } else {
        texte.bright_white().bold()
    }
}

/// Une couleur par famille de boutons : poings en bleu, pieds en rouge...
fn bouton(action: Action) -> ColoredString {
    let nom = action.to_string();
    match action {
        Action::LP | Action::MP | Action::HP => nom.bright_blue(),
        Action::LK | Action::MK | Action::HK => nom.bright_red(),
        Action::DP => nom.bright_green(),
        Action::DI => nom.yellow(),
        Action::Throw => nom.bright_magenta(),
    }
}

/// La ligne du trial : étapes faites en vert, étape attendue en jaune, le reste atténué.
///
///   TRIAL  ✓4HP xx ✓DR xx [4MK] xx 236MK, 4MK xx 63214MP
pub fn trial(t: &Trial) -> String {
    let mut texte = format!("{}  ", "TRIAL".bold().reversed());
    for (i, etape) in t.etapes.iter().enumerate() {
        match etape.liaison {
            Liaison::Debut => {}
            Liaison::Cancel => texte.push_str(&format!("{}", " xx ".dimmed())),
            Liaison::Link => texte.push_str(&format!("{}", ", ".dimmed())),
        }
        let n = &etape.notation;
        let morceau = if i < t.index {
            format!("✓{n}").bright_green()
        } else if i == t.index {
            format!("[{n}]").bright_yellow().bold()
        } else {
            n.dimmed()
        };
        texte.push_str(&format!("{morceau}"));
    }
    texte
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ligne_sans_couleur() {
        colored::control::set_override(false);
        let etat = Etat {
            direction: 2,
            boutons: vec![Action::MK],
        };
        let texte = ligne(&etat, 4, &[Action::MK], &["2MK".to_string()]);
        assert_eq!(texte, "  4  ↓ 2  MK    ► 2MK");

        let neutre = Etat {
            direction: 5,
            boutons: vec![],
        };
        assert_eq!(ligne(&neutre, 250, &[], &[]), "99+  • 5");
    }

    #[test]
    fn ligne_du_trial() {
        colored::control::set_override(false);
        let mut t = Trial::new("test", crate::combo::parser("4HP xx DR, 4MK").unwrap());
        t.index = 1;
        assert_eq!(trial(&t), "TRIAL  ✓4HP xx [DR], 4MK");
    }
}
