//! Reconnaissance des mouvements spéciaux (236, 623...) à partir de l'historique des directions.
//!
//! Idée : quand un bouton est appuyé, on regarde les directions des dernières frames
//! et on cherche si elles contiennent, dans l'ordre, les directions d'un mouvement.
//! Les directions en trop au milieu sont tolérées (2, 1, 2, 3, 6 compte comme un 236),
//! comme dans le jeu.

use std::collections::VecDeque;

/// Un mouvement spécial, nommé en notation numpad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// Demi-cercle avant
    M41236,
    /// Demi-cercle arrière
    M63214,
    /// Dragon punch (Shoryuken)
    M623,
    /// Dragon punch arrière
    M421,
    /// Quart de cercle avant (Hadoken)
    M236,
    /// Quart de cercle arrière (Tatsu)
    M214,
    /// Dash avant (sert au Drive Rush). Pas de bouton : il n'est pas dans `TOUS`.
    M66,
}

impl Motion {
    /// Ordre de priorité : on teste les mouvements longs d'abord,
    /// sinon un 41236 serait toujours reconnu comme un simple 236.
    /// Le 623 passe avant le 236, comme dans SF6.
    pub const TOUS: [Motion; 6] = [
        Motion::M41236,
        Motion::M63214,
        Motion::M623,
        Motion::M421,
        Motion::M236,
        Motion::M214,
    ];

    /// Les directions obligatoires, dans l'ordre.
    /// Pour les demi-cercles, SF6 accepte de sauter les diagonales :
    /// 6, 2, 1, 4 ou même 6, 2, 4 donnent un 63214. On ne demande donc que 6, 2, 4.
    pub fn directions(self) -> &'static [u8] {
        match self {
            Motion::M41236 => &[4, 2, 6],
            Motion::M63214 => &[6, 2, 4],
            Motion::M623 => &[6, 2, 3],
            Motion::M421 => &[4, 2, 1],
            Motion::M236 => &[2, 3, 6],
            Motion::M214 => &[2, 1, 4],
            Motion::M66 => &[6, 5, 6],
        }
    }

    /// Nombre de frames maximum entre le début du mouvement et l'appui du bouton.
    /// Valeurs de départ, à ajuster en jouant.
    pub fn fenetre(self) -> u64 {
        match self {
            Motion::M41236 | Motion::M63214 => 25,
            Motion::M66 => 12,
            _ => 18,
        }
    }

    /// Le nom affiché, ex. "236".
    pub fn nom(self) -> &'static str {
        match self {
            Motion::M41236 => "41236",
            Motion::M63214 => "63214",
            Motion::M623 => "623",
            Motion::M421 => "421",
            Motion::M236 => "236",
            Motion::M214 => "214",
            Motion::M66 => "66",
        }
    }
}

/// Les derniers changements de direction : (frame, direction numpad).
#[derive(Debug, Default)]
pub struct Historique {
    entrees: VecDeque<(u64, u8)>,
}

impl Historique {
    /// Note une nouvelle direction à la frame donnée.
    pub fn ajouter(&mut self, frame: u64, direction: u8) {
        if self.entrees.back().is_some_and(|&(_, d)| d == direction) {
            return;
        }
        self.entrees.push_back((frame, direction));
        // 1 seconde d'historique suffit largement. On garde quand même la dernière
        // direction d'avant cette seconde : elle peut être encore tenue (accroupi depuis longtemps).
        while let Some(&(suivante, _)) = self.entrees.get(1) {
            if frame.saturating_sub(suivante) > 60 {
                self.entrees.pop_front();
            } else {
                break;
            }
        }
    }

    /// Cherche le mouvement fait juste avant un appui de bouton à la frame `frame`.
    /// (Le trial utilise `reconnaitre_tous` ; celle-ci sert aux tests.)
    #[allow(dead_code)]
    pub fn reconnaitre(&self, frame: u64) -> Option<Motion> {
        Motion::TOUS.into_iter().find(|&m| self.contient(m, frame))
    }

    /// Tous les mouvements qui collent, pas seulement le prioritaire.
    /// Utile pour les trials : après un 4MK, un 236 contient aussi un 4, 2, 6 (41236).
    pub fn reconnaitre_tous(&self, frame: u64) -> Vec<Motion> {
        Motion::TOUS
            .into_iter()
            .filter(|&m| self.contient(m, frame))
            .collect()
    }

    /// Vrai si on vient de faire un dash avant (6, 5, 6).
    pub fn dash(&self, frame: u64) -> bool {
        self.contient(Motion::M66, frame)
    }

    fn contient(&self, motion: Motion, frame: u64) -> bool {
        let limite = frame.saturating_sub(motion.fenetre());
        let directions = motion.directions();
        // On remonte le temps : on cherche d'abord la dernière direction du mouvement,
        // puis l'avant-dernière, etc.
        let mut restantes = directions.len();
        for &(f, d) in self.entrees.iter().rev() {
            if d == directions[restantes - 1] {
                restantes -= 1;
                if restantes == 0 {
                    return true;
                }
            }
            // Cette direction a commencé avant la fenêtre : on ne remonte pas plus loin.
            // (On la compte quand même : elle était tenue au début de la fenêtre.)
            if f < limite {
                break;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construit un historique à partir de (frame, direction).
    fn historique(entrees: &[(u64, u8)]) -> Historique {
        let mut h = Historique::default();
        for &(f, d) in entrees {
            h.ajouter(f, d);
        }
        h
    }

    #[test]
    fn hadoken_propre() {
        let h = historique(&[(100, 2), (103, 3), (106, 6)]);
        assert_eq!(h.reconnaitre(108), Some(Motion::M236));
    }

    #[test]
    fn hadoken_avec_directions_en_trop() {
        let h = historique(&[(100, 2), (101, 1), (102, 2), (104, 3), (106, 6), (108, 5)]);
        assert_eq!(h.reconnaitre(109), Some(Motion::M236));
    }

    #[test]
    fn hadoken_trop_lent() {
        let h = historique(&[(100, 2), (110, 3), (125, 6)]);
        assert_eq!(h.reconnaitre(130), None);
    }

    #[test]
    fn bas_tenu_longtemps_puis_hadoken() {
        // Accroupi depuis longtemps, puis 3, 6 : c'est un 236 valide.
        let h = historique(&[(10, 2), (200, 3), (203, 6)]);
        assert_eq!(h.reconnaitre(205), Some(Motion::M236));
    }

    #[test]
    fn shoryuken() {
        let h = historique(&[(100, 6), (103, 2), (105, 3)]);
        assert_eq!(h.reconnaitre(107), Some(Motion::M623));
    }

    #[test]
    fn tatsu() {
        let h = historique(&[(100, 2), (102, 1), (104, 4)]);
        assert_eq!(h.reconnaitre(106), Some(Motion::M214));
    }

    #[test]
    fn demi_cercle_avant_prioritaire_sur_236() {
        let h = historique(&[(100, 4), (102, 1), (104, 2), (106, 3), (108, 6)]);
        assert_eq!(h.reconnaitre(110), Some(Motion::M41236));
    }

    #[test]
    fn demi_cercle_arriere_sans_la_diagonale_3() {
        // Ton input de Dhalsim : 6 (3 f), 2 (3 f), 1 (1 f), 4 (2 f), neutre, puis HP.
        // Le jeu sort bien le 63214HP, on doit le reconnaître aussi (et pas un 214).
        let h = historique(&[(100, 6), (103, 2), (106, 1), (107, 4), (109, 5)]);
        assert_eq!(h.reconnaitre(111), Some(Motion::M63214));
    }

    #[test]
    fn demi_cercle_arriere_complet() {
        let h = historique(&[(100, 6), (102, 3), (104, 2), (106, 1), (108, 4)]);
        assert_eq!(h.reconnaitre(110), Some(Motion::M63214));
    }

    #[test]
    fn dash_avant() {
        let h = historique(&[(100, 6), (103, 5), (106, 6)]);
        assert!(h.dash(106));
        let lent = historique(&[(100, 6), (110, 5), (125, 6)]);
        assert!(!lent.dash(125));
    }

    #[test]
    fn reconnaitre_tous_garde_le_236_apres_un_4() {
        let h = historique(&[(100, 4), (104, 2), (106, 3), (108, 6)]);
        let tous = h.reconnaitre_tous(110);
        assert_eq!(h.reconnaitre(110), Some(Motion::M41236));
        assert!(tous.contains(&Motion::M236));
    }

    #[test]
    fn un_retour_arriere_de_moins_d_une_frame_compte() {
        // 6, 2, 1, puis ← tenu moins d'une frame avant le neutre : même frame que le 5.
        let h = historique(&[(0, 6), (3, 2), (5, 1), (7, 4), (7, 5)]);
        assert!(h.reconnaitre_tous(10).contains(&Motion::M63214));
    }

    #[test]
    fn rien_quand_on_marche() {
        let h = historique(&[(100, 6), (130, 5)]);
        assert_eq!(h.reconnaitre(131), None);
    }

    #[test]
    fn ton_log_frame_3109() {
        // Extrait de ton log : 2, 1, 4, 5, 6 -> un 214 (puis on marche en avant).
        let h = historique(&[(3109, 2), (3110, 1), (3113, 4), (3114, 5)]);
        assert_eq!(h.reconnaitre(3114), Some(Motion::M214));
    }
}
