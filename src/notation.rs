//! Traduction des directions en notation numpad (1 à 9).
//!
//!   7 8 9
//!   4 5 6
//!   1 2 3
//!
//! 5 = neutre. On suppose que le joueur est à gauche (il regarde vers la droite) :
//! 6 = avant, 4 = arrière. Le côté droit sera géré plus tard en inversant 4 et 6.

/// L'état des 4 directions à un instant donné.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Directions {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

impl Directions {
    /// Renvoie le chiffre numpad correspondant.
    /// Gauche + droite ou haut + bas en même temps (SOCD) comptent comme neutre sur cet axe.
    /// Un leverless nettoie normalement le SOCD lui-même, c'est juste une sécurité.
    pub fn numpad(&self) -> u8 {
        let horizontal = match (self.left, self.right) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        let vertical = match (self.down, self.up) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        // 5 au centre, +1 vers la droite, +3 vers le haut.
        (5 + horizontal + 3 * vertical) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(up: bool, down: bool, left: bool, right: bool) -> Directions {
        Directions {
            up,
            down,
            left,
            right,
        }
    }

    #[test]
    fn neutre() {
        assert_eq!(Directions::default().numpad(), 5);
    }

    #[test]
    fn les_huit_directions() {
        assert_eq!(dirs(false, true, true, false).numpad(), 1);
        assert_eq!(dirs(false, true, false, false).numpad(), 2);
        assert_eq!(dirs(false, true, false, true).numpad(), 3);
        assert_eq!(dirs(false, false, true, false).numpad(), 4);
        assert_eq!(dirs(false, false, false, true).numpad(), 6);
        assert_eq!(dirs(true, false, true, false).numpad(), 7);
        assert_eq!(dirs(true, false, false, false).numpad(), 8);
        assert_eq!(dirs(true, false, false, true).numpad(), 9);
    }

    #[test]
    fn socd_gauche_droite_donne_neutre() {
        assert_eq!(dirs(false, false, true, true).numpad(), 5);
        assert_eq!(dirs(false, true, true, true).numpad(), 2);
    }
}
