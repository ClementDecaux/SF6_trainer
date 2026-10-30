//! Les combos : lecture de la notation et moteur de trial.
//!
//! Un combo s'écrit comme sur les forums, par exemple :
//! `b+hp xx DR xx b+mk xx qcf+mk, b+mk xx hcb+mp`
//! - `xx` ou `>` = cancel, `,` = link.
//! - b, f, d, u, db, df, cr. = directions ; qcf, qcb, hcf, hcb, dp = mouvements.
//! - DI = Drive Impact (HP+HK), DP = Drive Parry (MP+MK), DR = Drive Rush (66, ou MP+MK pendant un coup annulable).
//! - On peut aussi écrire directement en numpad : `4HP xx DI xx 4MK xx 236MK`.
//!
//! Chaque étape est ramenée à une forme unique (« 4HP », « 236MK », « DI »)
//! pour la comparer aux coups reconnus par l'outil.

use serde::Deserialize;

/// Comment une étape s'enchaîne avec la précédente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liaison {
    Debut,
    /// `xx` ou `>` : le coup suivant annule le précédent.
    Cancel,
    /// `,` : le coup suivant part après la fin du précédent.
    Link,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Etape {
    /// Forme unique : « 4HP », « 236MK », « DR »...
    pub notation: String,
    pub liaison: Liaison,
}

/// Une entrée du fichier `combos.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct ComboFichier {
    pub nom: String,
    pub inputs: String,
    /// Nombre de coups qui doivent toucher (sinon appris avec Select).
    #[serde(default)]
    pub hits: Option<u32>,
    /// Dégâts du combo complet (panneau « Combo Damage » du training). S'ils sont connus,
    /// c'est eux qui valident le combo (sinon appris avec Select).
    #[serde(default)]
    pub degats: Option<u32>,
    /// Le groupe du combo dans la liste de l'interface (« Drive Rush », « OD »...).
    #[serde(default)]
    pub categorie: String,
}

#[derive(Debug, Deserialize)]
pub struct Fichier {
    pub personnage: String,
    pub combo: Vec<ComboFichier>,
}

const FICHIER: &str = "combos.toml";

const COMBOS_PAR_DEFAUT: &str = include_str!("../combos_dhalsim.toml");

/// Charge `combos.toml`, ou le crée avec le combo d'exemple.
pub fn charger() -> Result<Fichier, String> {
    let texte = match std::fs::read_to_string(FICHIER) {
        Ok(texte) => texte,
        Err(_) => {
            std::fs::write(FICHIER, COMBOS_PAR_DEFAUT)
                .map_err(|e| format!("impossible de créer {FICHIER} : {e}"))?;
            println!("{FICHIER} créé avec un combo d'exemple.");
            COMBOS_PAR_DEFAUT.to_string()
        }
    };
    toml::from_str(&texte).map_err(|e| format!("erreur dans {FICHIER} : {e}"))
}

/// Le trial d'un combo du fichier, avec les hits et dégâts attendus (du fichier, sinon
/// appris avec Select dans `timings.toml`).
pub fn creer_trial(
    combo: &ComboFichier,
    timings: &crate::timings::Timings,
) -> Result<Trial, String> {
    let etapes = parser(&combo.inputs).map_err(|e| format!("{} : {e}", combo.nom))?;
    let mut trial = Trial::new(&combo.nom, etapes);
    trial.hits_fixe = combo.hits.is_some();
    trial.hits = combo.hits.or_else(|| timings.hits(&combo.nom));
    trial.degats_fixe = combo.degats.is_some();
    trial.degats = combo.degats.or_else(|| timings.degats(&combo.nom));
    Ok(trial)
}

/// Découpe un combo en étapes.
pub fn parser(texte: &str) -> Result<Vec<Etape>, String> {
    // « xx » devient « > » pour n'avoir que deux séparateurs à gérer.
    let texte = texte.to_lowercase().replace("xx", ">");
    let mut etapes = Vec::new();
    let mut liaison = Liaison::Debut;
    let mut morceau = String::new();

    // Le '\0' final sert à traiter le dernier morceau comme les autres.
    for c in texte.chars().chain(['\0']) {
        if c == '>' || c == ',' || c == '\0' {
            if morceau.trim().is_empty() {
                return Err(format!("étape vide dans « {texte} »"));
            }
            etapes.push(Etape {
                notation: normaliser(&morceau)?,
                liaison,
            });
            morceau.clear();
            liaison = if c == ',' {
                Liaison::Link
            } else {
                Liaison::Cancel
            };
        } else {
            morceau.push(c);
        }
    }
    Ok(etapes)
}

/// Ramène une étape à sa forme unique : « b+hp » -> « 4HP », « qcf+mk » -> « 236MK ».
pub fn normaliser(etape: &str) -> Result<String, String> {
    // Les remarques entre parenthèses (« (pc) », « (hit) ») sont ignorées.
    let mut sans_remarques = String::new();
    let mut profondeur = 0;
    for c in etape.chars() {
        match c {
            '(' => profondeur += 1,
            ')' => profondeur -= 1,
            _ if profondeur == 0 => sans_remarques.push(c),
            _ => {}
        }
    }
    // Les flèches (« ↓↘→ HP ») sont changées en chiffres numpad.
    let t: String = sans_remarques
        .chars()
        .map(|c| match c {
            '↙' => '1',
            '↓' => '2',
            '↘' => '3',
            '←' => '4',
            '→' => '6',
            '↖' => '7',
            '↑' => '8',
            '↗' => '9',
            _ => c,
        })
        .collect::<String>()
        .to_uppercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '+' | '.' | '/'))
        .collect();

    // Les actions qui s'écrivent seules.
    match t.as_str() {
        "DR" | "66" => return Ok("DR".to_string()),
        "DI" | "DP" | "THROW" => return Ok(t),
        _ => {}
    }

    // Les abréviations en tête, les plus longues d'abord (« DF » avant « D »).
    const ABREVIATIONS: [(&str, &str); 15] = [
        ("QCF", "236"),
        ("QCB", "214"),
        ("HCF", "41236"),
        ("HCB", "63214"),
        ("DP", "623"),
        ("DF", "3"),
        ("DB", "1"),
        ("UF", "9"),
        ("UB", "7"),
        ("CR", "2"),
        ("ST", "5"),
        ("B", "4"),
        ("F", "6"),
        ("D", "2"),
        ("U", "8"),
    ];
    let mut reste = t.as_str();
    let mut mouvement = String::new();
    for (abr, numpad) in ABREVIATIONS {
        if let Some(apres) = reste.strip_prefix(abr) {
            mouvement = numpad.to_string();
            reste = apres;
            break;
        }
    }

    // Puis les chiffres éventuels (notation numpad déjà écrite).
    let chiffres: String = reste.chars().take_while(|c| c.is_ascii_digit()).collect();
    mouvement.push_str(&chiffres);
    let bouton = &reste[chiffres.len()..];
    if mouvement.is_empty() {
        mouvement = "5".to_string();
    }

    const MOUVEMENTS: [&str; 15] = [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "236", "214", "623", "421", "41236", "63214",
    ];
    // PP et KK : version OD (deux poings ou deux pieds ensemble).
    const BOUTONS: [&str; 8] = ["LP", "MP", "HP", "LK", "MK", "HK", "PP", "KK"];
    if !MOUVEMENTS.contains(&mouvement.as_str()) {
        return Err(format!(
            "mouvement inconnu « {mouvement} » dans « {etape} »"
        ));
    }
    if !BOUTONS.contains(&bouton) {
        return Err(format!("bouton inconnu « {bouton} » dans « {etape} »"));
    }
    Ok(format!("{mouvement}{bouton}"))
}

/// Au-delà de ce nombre de frames (5 s) sans étape validée, l'essai est considéré comme
/// abandonné et le trial recommence. Ce n'est pas un timing à respecter : seuls les coups
/// lus dans le jeu et ses hits valident le combo.
const DELAI_MAX: u64 = 300;

/// Un double tap (ou un mash) et un Drive Rush pas lu se font dans la foulée de l'étape
/// d'avant. Plus tard que ça, le même coup est plutôt le début d'un nouvel essai.
const FOULEE_MAX: u64 = 50;

/// Ce qui arrive quand on donne un coup au trial.
#[derive(Debug, PartialEq, Eq)]
pub enum Resultat {
    /// Bonne étape, on passe à la suivante.
    Avance,
    /// Dernière étape validée.
    Reussi,
    /// Mauvais coup : on recommence.
    Rate { attendu: String },
    /// Coup sans effet (ex. le HP d'un HP+HK pour le Drive Impact).
    Ignore,
}

/// Ce qui arrive à l'essai quand le jeu dit que le combo s'est arrêté.
#[derive(Debug, PartialEq, Eq)]
pub enum Recalage {
    /// Recalé sur un nouveau départ : le numéro du premier appui gardé.
    Repris(usize),
    /// Remis à zéro : on attend la 1re étape.
    Remis,
}

/// Un trial en cours.
pub struct Trial {
    pub nom: String,
    pub etapes: Vec<Etape>,
    /// Index de l'étape attendue.
    pub index: usize,
    derniere_frame: u64,
    /// Frame du dernier appui pris en compte (étape validée, ou mash ignoré).
    dernier_appui: u64,
    /// Essai en cours (ou dernier essai) : écart en frames de chaque étape avec la précédente.
    pub ecarts: Vec<u64>,
    /// Appuis ignorés car faits pendant la récupération (coups intercalés, mash).
    pub intercales: u32,
    /// Nombre de coups qui touchent quand le combo passe (combos.toml, ou appris avec Select).
    pub hits: Option<u32>,
    /// `hits` vient de combos.toml : Select ne le change pas.
    pub hits_fixe: bool,
    /// Dégâts du combo complet (combos.toml, ou appris avec Select).
    pub degats: Option<u32>,
    pub degats_fixe: bool,
    /// De quel côté de l'écran on joue (réglage).
    pub cote: Cote,
    /// Essai en cours joué à droite : les coups sont retournés (4 <-> 6) avant d'être comparés.
    pub miroir: bool,
    /// Les appuis qui ont validé une étape dans l'essai en cours (lectures, frame).
    appuis: Vec<(Vec<String>, u64)>,
    /// Après un raté : l'essai a été recalé sur un nouveau départ pris parmi les appuis de
    /// l'essai raté (le numéro du premier appui gardé). Voir [`Trial::coup`].
    pub reprise: Option<usize>,
}

/// Le côté de l'écran du joueur. À droite, « avant » est vers la gauche : un 236 fait au
/// stick est un 214 à l'écran.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cote {
    Gauche,
    Droite,
    /// Le premier coup de chaque essai dit de quel côté on est.
    #[default]
    Auto,
}

/// Le coup vu de l'autre côté de l'écran : 1 <-> 3, 4 <-> 6, 7 <-> 9 (236 <-> 214...).
/// Le dash avant (DR) devient un dash arrière (44), et inversement.
pub fn miroir(coup: &str) -> String {
    match coup {
        "DR" => return "44".to_string(),
        "44" => return "DR".to_string(),
        _ => {}
    }
    let chiffres = coup.chars().take_while(|c| c.is_ascii_digit()).count();
    let retourne: String = coup[..chiffres]
        .chars()
        .map(|c| match c {
            '1' => '3',
            '3' => '1',
            '4' => '6',
            '6' => '4',
            '7' => '9',
            '9' => '7',
            _ => c,
        })
        .collect();
    format!("{retourne}{}", &coup[chiffres..])
}

impl Trial {
    pub fn new(nom: &str, etapes: Vec<Etape>) -> Trial {
        Trial {
            nom: nom.to_string(),
            etapes,
            index: 0,
            derniere_frame: 0,
            dernier_appui: 0,
            ecarts: Vec::new(),
            intercales: 0,
            hits: None,
            hits_fixe: false,
            degats: None,
            cote: Cote::Auto,
            miroir: false,
            degats_fixe: false,
            appuis: Vec::new(),
            reprise: None,
        }
    }

    /// Le même trial, sans essai en cours.
    fn vierge(&self) -> Trial {
        let mut t = Trial::new(&self.nom, self.etapes.clone());
        t.hits = self.hits;
        t.hits_fixe = self.hits_fixe;
        t.degats = self.degats;
        t.degats_fixe = self.degats_fixe;
        t.cote = self.cote;
        t
    }

    /// Un nouvel essai commence (1re étape validée).
    fn nouvel_essai(&mut self) {
        self.ecarts.clear();
        self.intercales = 0;
    }

    /// Début d'un essai : de quel côté joue-t-on ? En auto, le côté où le coup est le bon.
    fn choisir_cote(&mut self, bruts: &[String], retournes: &[String]) {
        let attendu = self.attendu();
        self.miroir = match self.cote {
            Cote::Gauche => false,
            Cote::Droite => true,
            Cote::Auto => {
                !bruts.iter().any(|c| self.accepte(c, attendu))
                    && retournes.iter().any(|c| self.accepte(c, attendu))
            }
        };
    }

    pub fn attendu(&self) -> &str {
        &self.etapes[self.index].notation
    }

    /// Un coup vient d'être fait. `candidats` = toutes les lectures possibles
    /// (ex. « 236MK » et « 41236MK » pour le même input).
    /// Un coup (toutes ses lectures possibles) donné au trial.
    ///
    /// Sur un raté, on regarde si les derniers appuis forment le début du combo : c'est
    /// qu'on a recommencé le combo avec un coup qui revient plus loin dedans (« b+MK, DR,
    /// d/b+HP, b+MK… » : le b+MK du nouveau départ ressemble à la 4e étape). Le trial est
    /// alors recalé sur ce nouveau départ (`reprise`), et le raté de l'ancien essai reste.
    pub fn coup(&mut self, candidats: &[String], frame: u64) -> Resultat {
        self.reprise = None;
        let avant = std::mem::take(&mut self.appuis);
        let resultat = self.coup_simple(candidats, frame);
        let appui = (candidats.to_vec(), frame);
        match &resultat {
            Resultat::Avance if self.index == 1 => self.appuis = vec![appui],
            Resultat::Avance => {
                self.appuis = avant;
                self.appuis.push(appui);
            }
            Resultat::Ignore => self.appuis = avant,
            Resultat::Reussi => {}
            Resultat::Rate { .. } => {
                if let Some((debut, trial)) = self.recaler(&avant, &appui) {
                    let (cote, miroir) = (self.cote, trial.miroir);
                    *self = trial;
                    self.cote = cote;
                    self.miroir = miroir;
                    self.reprise = Some(debut);
                } else if self.index == 1 {
                    self.appuis = vec![appui];
                }
            }
        }
        resultat
    }

    /// Le jeu dit que le combo s'est arrêté (ses dégâts sont repartis de zéro) : les appuis
    /// faits depuis la frame `frame_min` (après le dernier coup qui a touché) appartiennent à
    /// un nouveau combo. S'ils forment le début du combo, l'essai est recalé dessus
    /// (`reprise`), sinon il est remis à zéro. Renvoie `None` si rien ne change (pas d'essai
    /// en cours, ou tout l'essai est déjà dans le nouveau combo).
    pub fn combo_arrete(&mut self, frame_min: u64) -> Option<Recalage> {
        if self.index == 0 {
            return None;
        }
        let debut = self.appuis.iter().position(|(_, f)| *f >= frame_min);
        if debut == Some(0) {
            return None;
        }
        if let Some(debut) = debut {
            let mut essai = self.vierge();
            let appuis = self.appuis[debut..].to_vec();
            if appuis
                .iter()
                .all(|(c, f)| essai.coup_simple(c, *f) == Resultat::Avance)
            {
                essai.appuis = appuis;
                let cote = self.cote;
                *self = essai;
                self.cote = cote;
                self.reprise = Some(debut);
                return Some(Recalage::Repris(debut));
            }
        }
        self.index = 0;
        self.appuis.clear();
        Some(Recalage::Remis)
    }

    /// Le plus long nouveau départ possible parmi les appuis de l'essai raté (sans le 1er) :
    /// rejoués avec l'appui raté, ils doivent tous valider une étape.
    fn recaler(
        &self,
        avant: &[(Vec<String>, u64)],
        rate: &(Vec<String>, u64),
    ) -> Option<(usize, Trial)> {
        (1..avant.len()).find_map(|debut| {
            let mut essai = self.vierge();
            let appuis: Vec<_> = avant[debut..].iter().chain([rate]).cloned().collect();
            for (candidats, frame) in &appuis {
                if essai.coup_simple(candidats, *frame) != Resultat::Avance {
                    return None;
                }
            }
            essai.appuis = appuis;
            Some((debut, essai))
        })
    }

    fn coup_simple(&mut self, candidats: &[String], frame: u64) -> Resultat {
        // Pas de fenêtre de timing : le coup attendu fait avancer, quel que soit le moment.
        // Ce sont les hits lus dans le jeu qui disent si le combo est passé.
        let ecart = frame.saturating_sub(self.derniere_frame);

        // Joué à droite : on compare les coups retournés.
        let retournes: Vec<String> = candidats.iter().map(|c| miroir(c)).collect();
        if self.index == 0 {
            self.choisir_cote(candidats, &retournes);
        }
        let candidats: &[String] = if self.miroir { &retournes } else { candidats };

        if candidats.iter().any(|c| self.accepte(c, self.attendu())) {
            if self.index == 0 {
                self.nouvel_essai();
            } else {
                self.ecarts.push(ecart);
            }
            self.index += 1;
            self.derniere_frame = frame;
            self.dernier_appui = frame;
            if self.index == self.etapes.len() {
                self.index = 0;
                return Resultat::Reussi;
            }
            return Resultat::Avance;
        }

        // Drive Rush pas lu (son appui MP+MK n'apparaît pas toujours dans l'historique du
        // jeu) mais le coup d'après est là : on le compte comme fait. Ce sont les hits du jeu
        // qui diront si le combo est vraiment passé. Seulement dans la foulée : plus tard,
        // c'est sans doute qu'on recommence le combo (« LP, DR, LP » raté après le 1er LP).
        if self.hits.is_some()
            && self.index > 0
            && ecart <= FOULEE_MAX
            && self.attendu() == "DR"
            && let Some(apres) = self.etapes.get(self.index + 1).map(|e| e.notation.clone())
            && candidats.iter().any(|c| self.accepte(c, &apres))
        {
            self.ecarts.push(ecart);
            self.ecarts.push(0);
            self.index += 2;
            self.derniere_frame = frame;
            self.dernier_appui = frame;
            if self.index == self.etapes.len() {
                self.index = 0;
                return Resultat::Reussi;
            }
            return Resultat::Avance;
        }

        // Pour un Drive Impact (HP+HK) ou un Drive Parry (MP+MK), le premier des
        // deux boutons arrive seul : on attend le second au lieu de compter un raté.
        // Pareil pour un coup OD (PP ou KK) : le premier poing (ou pied) arrive seul.
        let boutons: &[&str] = match self.attendu() {
            "DI" => &["HP", "HK"],
            "DP" | "DR" => &["MP", "MK"],
            a if a.ends_with("PP") => &["LP", "MP", "HP"],
            a if a.ends_with("KK") => &["LK", "MK", "HK"],
            _ => &[],
        };
        if !boutons.is_empty()
            && candidats
                .iter()
                .all(|c| boutons.iter().any(|b| c.ends_with(b)))
        {
            return Resultat::Ignore;
        }

        // Rien de commencé : on ne compte pas de raté, on attend la première étape.
        if self.index == 0 {
            return Resultat::Ignore;
        }

        // Le même bouton que l'étape d'avant, ré-appuyé : c'est le « double tap » (ou le mash)
        // qu'on fait pour assurer un link. Le jeu l'ignore pendant la récupération, nous aussi.
        // La direction ne compte pas : en mashant, le stick revient souvent au neutre (5MK).
        // Seulement dans la foulée : plus tard, ce coup recommence le combo.
        let bouton_avant = bouton(&self.etapes[self.index - 1].notation);
        if frame.saturating_sub(self.dernier_appui) <= FOULEE_MAX
            && candidats.iter().any(|c| bouton(c) == bouton_avant)
        {
            self.intercales += 1;
            self.dernier_appui = frame;
            return Resultat::Ignore;
        }
        let attendu = self.attendu().to_string();
        self.index = 0;
        // Ce coup raté est peut-être le début d'un nouvel essai (peut-être de l'autre côté).
        let (bruts, retournes): (Vec<String>, Vec<String>) = if self.miroir {
            (retournes_de(candidats), candidats.to_vec())
        } else {
            (candidats.to_vec(), retournes_de(candidats))
        };
        self.choisir_cote(&bruts, &retournes);
        let candidats: &[String] = if self.miroir { &retournes } else { &bruts };
        if candidats.iter().any(|c| self.accepte(c, self.attendu())) {
            self.nouvel_essai();
            self.index = 1;
            self.derniere_frame = frame;
            self.dernier_appui = frame;
        }
        Resultat::Rate { attendu }
    }

    /// Ce coup valide-t-il l'étape `attendu` ? Quand les hits sont vérifiés dans le jeu
    /// (`hits` connu), la direction exacte d'un coup normal ne compte pas : 5MK ou 6MK au
    /// lieu de 4MK peut sortir le même coup ou passer quand même, et le compteur de hits dira
    /// si ça a touché. Mais la posture compte : accroupi (1, 2, 3) et debout (4 à 9) sont
    /// deux coups différents (d/b HP n'est pas b+HP).
    pub fn accepte(&self, coup: &str, attendu: &str) -> bool {
        correspond(coup, attendu)
            || (self.hits.is_some()
                && normal(coup)
                && normal(attendu)
                && bouton(coup) == bouton(attendu)
                && accroupi(coup) == accroupi(attendu))
    }

    /// À appeler régulièrement : remet à zéro si on a trop attendu. Renvoie vrai si c'est le cas.
    pub fn temps_depasse(&mut self, frame: u64) -> bool {
        if self.index > 0 && frame.saturating_sub(self.derniere_frame) > DELAI_MAX {
            self.index = 0;
            return true;
        }
        false
    }
}

/// Un coup reconnu valide-t-il l'étape attendue ? En plus du nom exact : pendant un coup
/// annulable, MP+MK (le bouton Drive Parry) lance un Drive Rush, comme 66.
pub fn correspond(coup: &str, attendu: &str) -> bool {
    coup == attendu || (attendu == "DR" && coup == "DP")
}

fn retournes_de(coups: &[String]) -> Vec<String> {
    coups.iter().map(|c| miroir(c)).collect()
}

/// Un coup normal : une direction et un bouton (« 4MK »), pas un mouvement spécial.
fn normal(notation: &str) -> bool {
    let chiffres = notation.chars().take_while(|c| c.is_ascii_digit()).count();
    chiffres == 1 && notation.len() > 1
}

/// Un coup normal accroupi (direction 1, 2 ou 3).
fn accroupi(notation: &str) -> bool {
    matches!(notation.chars().next(), Some('1' | '2' | '3'))
}

/// Le bouton d'une notation, sans la direction ni le mouvement : « 236MK » -> « MK ».
fn bouton(notation: &str) -> &str {
    notation.trim_start_matches(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Vec<String> {
        vec![s.to_string()]
    }

    #[test]
    fn drive_rush_pas_lu_mais_le_coup_suivant_est_la() {
        let etapes = parser("4HP xx DR xx 4MK xx 236MK").unwrap();
        let mut t = Trial::new("test", etapes.clone());
        t.hits = Some(4);
        assert_eq!(t.coup(&c("4HP"), 10), Resultat::Avance);
        assert_eq!(t.coup(&c("4MK"), 30), Resultat::Avance);
        assert_eq!(t.index, 3);
        assert_eq!(t.coup(&c("236MK"), 50), Resultat::Reussi);
        // Sans les hits du jeu pour vérifier, on attend le DR.
        let mut t = Trial::new("test", etapes);
        assert_eq!(t.coup(&c("4HP"), 10), Resultat::Avance);
        assert_eq!(t.coup(&c("4MK"), 30), Resultat::Ignore);
    }

    #[test]
    fn miroir_des_coups() {
        assert_eq!(miroir("236MK"), "214MK");
        assert_eq!(miroir("63214MP"), "41236MP");
        assert_eq!(miroir("4HP"), "6HP");
        assert_eq!(miroir("2MK"), "2MK");
        assert_eq!(miroir("DR"), "44");
        assert_eq!(miroir("44"), "DR");
        assert_eq!(miroir("DI"), "DI");
    }

    #[test]
    fn combo_joue_a_droite() {
        let etapes = parser("4HP xx DR xx 4MK xx 236MK").unwrap();
        // Auto : le 1er coup (6HP à l'écran = 4HP vu de droite) dit qu'on est à droite.
        let mut t = Trial::new("test", etapes.clone());
        assert_eq!(t.coup(&c("6HP"), 10), Resultat::Avance);
        assert!(t.miroir);
        assert_eq!(t.coup(&c("44"), 20), Resultat::Avance);
        assert_eq!(t.coup(&c("6MK"), 30), Resultat::Avance);
        assert_eq!(t.coup(&c("214MK"), 40), Resultat::Reussi);
        // Forcé à gauche : 6HP ne commence rien.
        let mut t = Trial::new("test", etapes.clone());
        t.cote = Cote::Gauche;
        assert_eq!(t.coup(&c("6HP"), 10), Resultat::Ignore);
        // Forcé à droite : 4HP ne commence rien, 6HP oui.
        let mut t = Trial::new("test", etapes);
        t.cote = Cote::Droite;
        assert_eq!(t.coup(&c("4HP"), 10), Resultat::Ignore);
        assert_eq!(t.coup(&c("6HP"), 12), Resultat::Avance);
    }

    #[test]
    fn les_fleches_sont_comprises() {
        assert_eq!(normaliser("↓↘→+MK").unwrap(), "236MK");
        assert_eq!(normaliser("← HP").unwrap(), "4HP");
    }

    #[test]
    fn drive_rush_par_le_bouton_parry() {
        let mut t = Trial::new("test", parser("4HP xx DR xx 4MK").unwrap());
        assert_eq!(t.coup(&c("4HP"), 10), Resultat::Avance);
        // MP seul (boutons séparés) : on attend le MK.
        assert_eq!(t.coup(&c("5MP"), 12), Resultat::Ignore);
        assert_eq!(t.coup(&c("DP"), 13), Resultat::Avance);
        assert_eq!(t.coup(&c("4MK"), 30), Resultat::Reussi);
    }

    #[test]
    fn direction_libre_si_les_hits_sont_verifies() {
        let mut t = Trial::new("test", parser("4HP xx 4MK xx 236MK").unwrap());
        assert_eq!(t.coup(&c("4HP"), 10), Resultat::Avance);
        // Sans vérification des hits : 6MK n'est pas 4MK.
        assert!(matches!(t.coup(&c("6MK"), 20), Resultat::Rate { .. }));
        t.hits = Some(3);
        assert_eq!(t.coup(&c("4HP"), 30), Resultat::Avance);
        assert_eq!(
            t.coup(&["41236MK".to_string(), "6MK".to_string()], 40),
            Resultat::Avance
        );
        // Un mouvement spécial doit toujours être fait.
        assert!(!t.accepte("2MK", "236MK"));
        assert!(t.accepte("5MK", "4MK"));
        // Accroupi et debout ne se remplacent pas.
        assert!(!t.accepte("1HP", "4HP"));
        assert!(!t.accepte("5MK", "2MK"));
        assert!(t.accepte("1MK", "2MK"));
    }

    #[test]
    fn ton_combo_dhalsim() {
        let etapes = parser("b+hp xx DI xx b+mk xx qcf+mk, b+mk xx hcb+mp").unwrap();
        let notations: Vec<&str> = etapes.iter().map(|e| e.notation.as_str()).collect();
        assert_eq!(notations, ["4HP", "DI", "4MK", "236MK", "4MK", "63214MP"]);
        assert_eq!(etapes[0].liaison, Liaison::Debut);
        assert_eq!(etapes[1].liaison, Liaison::Cancel);
        assert_eq!(etapes[4].liaison, Liaison::Link);
    }

    #[test]
    fn autres_ecritures() {
        assert_eq!(normaliser("cr.mk").unwrap(), "2MK");
        assert_eq!(normaliser("st.HP").unwrap(), "5HP");
        assert_eq!(normaliser("MP").unwrap(), "5MP");
        assert_eq!(normaliser("dp+lp").unwrap(), "623LP");
        assert_eq!(normaliser("DP").unwrap(), "DP");
        assert_eq!(normaliser("df+hk").unwrap(), "3HK");
        assert_eq!(normaliser("236 HP").unwrap(), "236HP");
        assert_eq!(normaliser("d/b+hp").unwrap(), "1HP");
        assert_eq!(normaliser("cr. lp (pc)").unwrap(), "2LP");
        assert_eq!(normaliser("hcb+kk").unwrap(), "63214KK");
        assert!(normaliser("qcf+xx").is_err());
        assert!(normaliser("25HP").is_err());
    }

    #[test]
    fn le_fichier_par_defaut_se_lit() {
        let f: Fichier = toml::from_str(COMBOS_PAR_DEFAUT).unwrap();
        assert_eq!(f.personnage, "Dhalsim");
        for c in &f.combo {
            assert!(
                parser(&c.inputs).is_ok(),
                "{} : {:?}",
                c.nom,
                parser(&c.inputs)
            );
        }
        assert_eq!(f.combo[0].degats, Some(2346));
    }

    #[test]
    fn trial_reussi_avec_di_par_hp_hk() {
        let mut t = Trial::new("test", parser("4HP xx DI xx 4MK").unwrap());
        assert_eq!(t.coup(&c("4HP"), 10), Resultat::Avance);
        // HP puis HK : le HP seul est ignoré, HK + HP tenu donne le DI.
        assert_eq!(t.coup(&c("5HP"), 20), Resultat::Ignore);
        assert_eq!(
            t.coup(&["5HK".to_string(), "DI".to_string()], 21),
            Resultat::Avance
        );
        assert_eq!(t.coup(&c("4MK"), 40), Resultat::Reussi);
        assert_eq!(t.index, 0);
    }

    #[test]
    fn coup_od_par_deux_poings() {
        let mut t = Trial::new("test", parser("b+mk xx hcb+pp").unwrap());
        assert_eq!(t.coup(&c("4MK"), 10), Resultat::Avance);
        // Le premier poing seul : on attend le second.
        assert_eq!(t.coup(&c("63214LP"), 20), Resultat::Ignore);
        assert_eq!(
            t.coup(&["63214MP".to_string(), "63214PP".to_string()], 21),
            Resultat::Reussi
        );
    }

    #[test]
    fn mauvais_coup_remet_a_zero() {
        let mut t = Trial::new("test", parser("4HP xx 236MK").unwrap());
        t.coup(&c("4HP"), 10);
        assert_eq!(
            t.coup(&c("236MP"), 20),
            Resultat::Rate {
                attendu: "236MK".to_string()
            }
        );
        assert_eq!(t.index, 0);
    }

    #[test]
    fn double_tap_sur_un_link() {
        // Ton log : 4MK validé, puis MK ré-appuyé deux fois avant le 63214MP.
        let mut t = Trial::new("test", parser("236MK, 4MK xx 63214MP").unwrap());
        t.coup(&c("236MK"), 10);
        assert_eq!(t.coup(&c("4MK"), 30), Resultat::Avance);
        assert_eq!(t.coup(&c("4MK"), 34), Resultat::Ignore);
        assert_eq!(t.coup(&c("4MK"), 40), Resultat::Ignore);
        assert_eq!(t.coup(&c("63214MP"), 50), Resultat::Reussi);
    }

    #[test]
    fn mash_qui_revient_au_neutre() {
        // Ton log : en mashant MK après le 4MK, le dernier appui sort en 5MK (stick au neutre).
        let mut t = Trial::new("test", parser("236MK, 4MK xx 63214MP").unwrap());
        t.coup(&c("236MK"), 10);
        assert_eq!(t.coup(&c("4MK"), 30), Resultat::Avance);
        assert_eq!(t.coup(&c("5MK"), 40), Resultat::Ignore);
        assert_eq!(t.coup(&c("63214MP"), 50), Resultat::Reussi);
    }

    #[test]
    fn on_recommence_lp_dr_lp_apres_une_erreur() {
        // « LP, DR, LP » : 1er LP, le Drive Rush ne sort pas, et on recommence le combo.
        let mut t = Trial::new("test", parser("5LP xx DR, 5LP").unwrap());
        t.hits = Some(2);
        assert_eq!(t.coup(&c("5LP"), 100), Resultat::Avance);
        // Le LP du nouvel essai, bien après : pas un mash, pas un DR sauté, un nouveau départ.
        assert!(matches!(t.coup(&c("5LP"), 200), Resultat::Rate { .. }));
        assert_eq!(t.index, 1);
        assert_eq!(t.coup(&c("DR"), 215), Resultat::Avance);
        assert_eq!(t.coup(&c("5LP"), 235), Resultat::Reussi);
    }

    #[test]
    fn recommencer_avec_un_coup_qui_revient_dans_le_combo() {
        // b+MK xx DR xx d/b+HP, b+MK xx qcb+MP : arrêté après d/b+HP, on recommence.
        let mut t = Trial::new("test", parser("4MK xx DR xx 1HP, 4MK xx 63214MP").unwrap());
        assert_eq!(t.coup(&c("4MK"), 100), Resultat::Avance);
        assert_eq!(t.coup(&c("DR"), 115), Resultat::Avance);
        assert_eq!(t.coup(&c("1HP"), 135), Resultat::Avance);
        // Le b+MK du nouveau départ passe pour la 4e étape…
        assert_eq!(t.coup(&c("4MK"), 300), Resultat::Avance);
        // … mais le DR qui suit montre qu'on a recommencé : recalé à l'étape 3.
        assert!(matches!(t.coup(&c("DR"), 315), Resultat::Rate { .. }));
        assert_eq!((t.index, t.reprise), (2, Some(3)));
        assert_eq!(t.ecarts, vec![15]);
        assert_eq!(t.coup(&c("1HP"), 335), Resultat::Avance);
        assert_eq!(t.coup(&c("4MK"), 360), Resultat::Avance);
        assert_eq!(t.coup(&c("63214MP"), 380), Resultat::Reussi);
    }

    #[test]
    fn le_jeu_dit_que_le_combo_est_fini() {
        let mut t = Trial::new("test", parser("4MK xx DR xx 1HP, 4MK xx 63214MP").unwrap());
        t.coup(&c("4MK"), 100);
        t.coup(&c("DR"), 115);
        t.coup(&c("1HP"), 135);
        t.coup(&c("4MK"), 300);
        // Dernier coup qui a touché vers la frame 150 : le b+MK de la frame 300 est un
        // nouveau combo.
        assert_eq!(t.combo_arrete(151), Some(Recalage::Repris(3)));
        assert_eq!(t.index, 1);
        assert_eq!(t.coup(&c("DR"), 315), Resultat::Avance);
        // Rien de neuf depuis : on attend la 1re étape.
        let mut t = Trial::new("test", parser("4MK xx DR xx 1HP").unwrap());
        t.coup(&c("4MK"), 100);
        t.coup(&c("DR"), 115);
        assert_eq!(t.combo_arrete(200), Some(Recalage::Remis));
        assert_eq!(t.index, 0);
        // Tout l'essai est déjà dans le nouveau combo : rien ne change.
        t.coup(&c("4MK"), 300);
        assert_eq!(t.combo_arrete(250), None);
        assert_eq!(t.index, 1);
    }

    #[test]
    fn un_vrai_rate_ne_recale_pas() {
        let mut t = Trial::new("test", parser("4MK xx DR xx 1HP, 4MK xx 63214MP").unwrap());
        t.coup(&c("4MK"), 100);
        t.coup(&c("DR"), 115);
        assert!(matches!(t.coup(&c("5LP"), 130), Resultat::Rate { .. }));
        assert_eq!((t.index, t.reprise), (0, None));
    }

    #[test]
    fn drive_rush_saute_seulement_dans_la_foulee() {
        let mut t = Trial::new("test", parser("5LP xx DR, 5LP").unwrap());
        t.hits = Some(2);
        t.coup(&c("5LP"), 100);
        // DR pas lu, le LP d'après arrive vite : combo fini.
        assert_eq!(t.coup(&c("5LP"), 125), Resultat::Reussi);
    }

    #[test]
    fn un_autre_bouton_reste_un_rate() {
        let mut t = Trial::new("test", parser("236MK, 4MK xx 63214MP").unwrap());
        t.coup(&c("236MK"), 10);
        t.coup(&c("4MK"), 30);
        assert!(matches!(t.coup(&c("5LP"), 40), Resultat::Rate { .. }));
    }

    #[test]
    fn plus_de_fenetre_de_timing() {
        let mut t = Trial::new("test", parser("4HP xx 236MK, 4MK").unwrap());
        t.coup(&c("4HP"), 100);
        // Tôt ou tard : le coup attendu avance, les écarts sont gardés pour information.
        assert_eq!(t.coup(&c("236MK"), 102), Resultat::Avance);
        assert_eq!(t.coup(&c("4MK"), 180), Resultat::Reussi);
        assert_eq!(t.ecarts, vec![2, 78]);
    }

    #[test]
    fn un_autre_coup_est_un_rate() {
        let mut t = Trial::new("test", parser("4HP xx 236MK, 4MK").unwrap());
        t.coup(&c("4HP"), 100);
        t.coup(&c("236MK"), 111);
        assert!(matches!(t.coup(&c("5LP"), 140), Resultat::Rate { .. }));
    }

    #[test]
    fn rien_ne_compte_avant_la_premiere_etape() {
        let mut t = Trial::new("test", parser("4HP xx 236MK").unwrap());
        assert_eq!(t.coup(&c("5LP"), 10), Resultat::Ignore);
    }

    #[test]
    fn essai_abandonne() {
        let mut t = Trial::new("test", parser("4HP, 4MK").unwrap());
        t.coup(&c("4HP"), 10);
        // Un link qui arrive 60 frames après n'est pas « trop lent ».
        assert!(!t.temps_depasse(70));
        assert!(!t.temps_depasse(200));
        assert!(t.temps_depasse(400));
        assert_eq!(t.index, 0);
    }
}
