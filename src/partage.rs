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
const LIGNES_MAX: usize = 100;

/// Une ligne de l'historique d'inputs (comme dans le terminal).
#[derive(Debug, Clone)]
pub struct Ligne {
    pub frames: u64,
    pub direction: u8,
    /// Boutons tenus, avec `true` s'ils viennent d'être appuyés.
    pub boutons: Vec<(Action, bool)>,
    pub coups: Vec<String>,
    /// Heure du début de l'état (horloge de la capture), pour le retrouver dans le jeu.
    pub debut: i64,
    /// Durée recalée sur l'historique du jeu (sinon : estimée avec notre horloge).
    pub exact: bool,
    /// Le combo a cassé sur cette ligne (affichée en rouge).
    pub cassee: bool,
    /// Le numéro (à partir de 1) de l'étape du combo validée sur cette ligne.
    pub etape: Option<usize>,
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
    /// Écart en frames de chaque étape avec la précédente (essai en cours ou dernier essai).
    pub ecarts: Vec<u64>,
    /// Mode déplacement (F8) : l'overlay peut être glissé à la souris.
    pub deplacement: bool,
    /// Les vidéos des combos réussis sont gardées (F9).
    pub enregistrement: bool,
    /// L'historique d'inputs de SF6 est lu à l'écran (frames exactes).
    pub historique_jeu: bool,
    /// Les écarts affichés viennent de l'historique du jeu.
    pub ecarts_exacts: bool,
    /// Pourquoi le dernier combo n'a pas pu être vérifié sur l'historique du jeu.
    pub raison_estimee: Option<String>,
    /// Le panneau des dégâts du jeu est lu (coups qui touchent comptés).
    pub degats_visibles: bool,
    /// Coups qui ont touché au dernier combo.
    pub hits: Option<u32>,
    /// Compteur de hits en direct (combo en cours ou dernier combo).
    pub hits_direct: u32,
    /// Nombre de lignes ajoutées à l'historique depuis le lancement.
    pub lignes_total: u64,
    /// Bouton photo cliqué sur l'overlay (le thread de la manette prend la photo).
    pub photo_demandee: bool,
    /// L'étape où le dernier essai a cassé (mauvais coup, hors timing ou coup qui n'a pas touché).
    pub etape_cassee: Option<usize>,
    /// Un mauvais coup vient d'être fait : la prochaine ligne ajoutée est celle du break.
    pub ligne_cassee_a_venir: bool,
    /// Une étape vient d'être validée : son numéro va sur la prochaine ligne ajoutée.
    pub etape_a_venir: Option<usize>,
    /// L'historique lu dans le jeu (lignes finies, la plus récente en premier),
    /// avec les coups reconnus de notre côté. Affiché quand l'historique du jeu est visible.
    pub lignes_jeu: VecDeque<Ligne>,
    /// La ligne en cours dans le jeu (sa durée vient du compteur affiché par le jeu).
    pub ligne_jeu_en_cours: Option<Ligne>,
    /// Nombre de lignes du jeu ajoutées depuis le lancement.
    pub lignes_jeu_total: u64,
    /// Les coups sont lus dans l'historique du jeu (sinon : sur la manette).
    pub source_jeu: bool,
    /// Coups lus dans le jeu : heures des appuis où le combo a cassé, à reporter sur
    /// les lignes du jeu.
    pub cassees_jeu: Vec<i64>,
    /// Les noms des combos de combos.toml, celui en cours, et celui demandé sur l'overlay.
    pub combos: Vec<String>,
    pub combo_index: usize,
    pub combo_demande: Option<usize>,
    /// L'overlay est affiché (bouton de l'interface).
    pub overlay_visible: bool,
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
            ecarts: Vec::new(),
            deplacement: false,
            enregistrement: false,
            historique_jeu: false,
            ecarts_exacts: false,
            raison_estimee: None,
            degats_visibles: false,
            hits: None,
            hits_direct: 0,
            lignes_total: 0,
            photo_demandee: false,
            etape_cassee: None,
            ligne_cassee_a_venir: false,
            etape_a_venir: None,
            lignes_jeu: VecDeque::new(),
            ligne_jeu_en_cours: None,
            lignes_jeu_total: 0,
            source_jeu: false,
            cassees_jeu: Vec::new(),
            combos: Vec::new(),
            combo_index: 0,
            combo_demande: None,
            overlay_visible: true,
        }
    }

    /// Une ligne finie de l'historique du jeu.
    pub fn ajouter_ligne_jeu(&mut self, ligne: Ligne) {
        self.lignes_jeu.push_front(ligne);
        self.lignes_jeu_total += 1;
        self.lignes_jeu.truncate(LIGNES_MAX);
    }

    pub fn ajouter_ligne(&mut self, mut ligne: Ligne) {
        // Coups lus dans le jeu : les marques vont sur les lignes du jeu, pas les nôtres.
        if !self.source_jeu {
            if std::mem::take(&mut self.ligne_cassee_a_venir) {
                ligne.cassee = true;
            }
            if let Some(etape) = self.etape_a_venir.take() {
                ligne.etape = Some(etape);
            }
        }
        self.lignes.push_front(ligne);
        self.lignes_total += 1;
        self.lignes.truncate(LIGNES_MAX);
    }

    /// Marque en rouge la ligne de l'appui fait à `temps` (celle où le combo a cassé).
    /// Si elle n'est pas encore dans l'historique (état encore tenu), ce sera la prochaine.
    pub fn marquer_cassee(&mut self, temps: i64) {
        // L'état d'un appui commence au plus tôt à sa frame : une frame de marge.
        const MARGE: i64 = 166_667;
        let proche = |l: &&mut Ligne| !l.coups.is_empty() && (l.debut - temps).abs() <= MARGE;
        let mut trouvee = false;
        // Dans les deux historiques : le nôtre et celui lu dans le jeu.
        for ligne in self
            .lignes
            .iter_mut()
            .find(proche)
            .into_iter()
            .chain(self.lignes_jeu.iter_mut().find(proche))
        {
            ligne.cassee = true;
            trouvee = true;
        }
        if self.source_jeu {
            // Coups lus dans le jeu : la ligne en cours est refaite à chaque nouvelle ligne,
            // la marque est gardée à part (voir `cassees_jeu`).
            self.cassees_jeu.push(temps);
            if let Some(l) = self
                .ligne_jeu_en_cours
                .as_mut()
                .filter(|l| (l.debut - temps).abs() <= MARGE)
            {
                l.cassee = true;
            }
        } else if !trouvee {
            self.ligne_cassee_a_venir = true;
        }
    }

    pub fn message(&mut self, texte: String, reussi: bool) {
        self.message = Some(Message {
            texte,
            reussi,
            quand: Instant::now(),
        });
    }
}
