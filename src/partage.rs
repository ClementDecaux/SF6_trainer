//! Ce que le thread de la manette partage avec l'overlay.
//!
//! Le thread de la manette écrit dedans, l'overlay le lit à chaque image.
//! Les deux y accèdent à travers un `Arc<Mutex<Partage>>` :
//! `Arc` pour que les deux threads possèdent la même donnée,
//! `Mutex` pour qu'un seul y touche à la fois.

use crate::combo::Etape;
use crate::controles::Action;
use std::collections::VecDeque;
use std::time::Instant;

/// Nombre de lignes d'historique gardées pour l'overlay.
const LIGNES_MAX: usize = 14;

/// Une ligne de l'historique d'inputs (comme dans le terminal).
#[derive(Debug, Clone)]
pub struct Ligne {
    pub frames: u64,
    pub direction: u8,
    /// Boutons tenus, avec `true` s'ils viennent d'être appuyés.
    pub boutons: Vec<(Action, bool)>,
    pub coups: Vec<String>,
}

/// Le dernier événement du trial, affiché quelques instants.
#[derive(Debug, Clone)]
pub struct Message {
    pub texte: String,
    pub reussi: bool,
    pub quand: Instant,
}

#[derive(Debug)]
pub struct Partage {
    pub nom: String,
    pub etapes: Vec<Etape>,
    /// Index de l'étape attendue.
    pub index: usize,
    pub message: Option<Message>,
    /// La plus récente en premier, comme dans SF6.
    pub lignes: VecDeque<Ligne>,
    /// Nombre de combos réussis depuis le lancement.
    pub reussites: u32,
}

impl Partage {
    pub fn new(nom: &str, etapes: Vec<Etape>) -> Partage {
        Partage {
            nom: nom.to_string(),
            etapes,
            index: 0,
            message: None,
            lignes: VecDeque::new(),
            reussites: 0,
        }
    }

    pub fn ajouter_ligne(&mut self, ligne: Ligne) {
        self.lignes.push_front(ligne);
        self.lignes.truncate(LIGNES_MAX);
    }

    pub fn message(&mut self, texte: String, reussi: bool) {
        self.message = Some(Message {
            texte,
            reussi,
            quand: Instant::now(),
        });
    }
}
