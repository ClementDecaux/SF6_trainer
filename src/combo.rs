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

/// Au-delà de ce nombre de frames sans étape validée, le trial recommence.
const DELAI_MAX: u64 = 90;

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

/// Un trial en cours.
pub struct Trial {
    pub nom: String,
    pub etapes: Vec<Etape>,
    /// Index de l'étape attendue.
    pub index: usize,
    derniere_frame: u64,
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
}

impl Trial {
    pub fn new(nom: &str, etapes: Vec<Etape>) -> Trial {
        Trial {
            nom: nom.to_string(),
            etapes,
            index: 0,
            derniere_frame: 0,
            ecarts: Vec::new(),
            intercales: 0,
            hits: None,
            hits_fixe: false,
            degats: None,
            degats_fixe: false,
        }
    }

    /// Un nouvel essai commence (1re étape validée).
    fn nouvel_essai(&mut self) {
        self.ecarts.clear();
        self.intercales = 0;
    }

    pub fn attendu(&self) -> &str {
        &self.etapes[self.index].notation
    }

    /// Un coup vient d'être fait. `candidats` = toutes les lectures possibles
    /// (ex. « 236MK » et « 41236MK » pour le même input).
    pub fn coup(&mut self, candidats: &[String], frame: u64) -> Resultat {
        // Pas de fenêtre de timing : le coup attendu fait avancer, quel que soit le moment.
        // Ce sont les hits lus dans le jeu qui disent si le combo est passé.
        let ecart = frame.saturating_sub(self.derniere_frame);

        if candidats.iter().any(|c| self.accepte(c, self.attendu())) {
            if self.index == 0 {
                self.nouvel_essai();
            } else {
                self.ecarts.push(ecart);
            }
            self.index += 1;
            self.derniere_frame = frame;
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
        let bouton_avant = bouton(&self.etapes[self.index - 1].notation);
        if candidats.iter().any(|c| bouton(c) == bouton_avant) {
            self.intercales += 1;
            return Resultat::Ignore;
        }
        let attendu = self.attendu().to_string();
        self.index = 0;
        // Ce coup raté est peut-être le début d'un nouvel essai.
        if candidats.iter().any(|c| self.accepte(c, self.attendu())) {
            self.nouvel_essai();
            self.index = 1;
            self.derniere_frame = frame;
        }
        Resultat::Rate { attendu }
    }

    /// Ce coup valide-t-il l'étape `attendu` ? Quand les hits sont vérifiés dans le jeu
    /// (`hits` connu), la direction d'un coup normal ne compte pas : 5MK ou 6MK au lieu de
    /// 4MK peut sortir le même coup (pas de 6MK chez ce perso) ou passer quand même. Si ça ne
    /// touche pas, le compteur de hits le dira.
    pub fn accepte(&self, coup: &str, attendu: &str) -> bool {
        correspond(coup, attendu)
            || (self.hits.is_some()
                && normal(coup)
                && normal(attendu)
                && bouton(coup) == bouton(attendu))
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

/// Un coup normal : une direction et un bouton (« 4MK »), pas un mouvement spécial.
fn normal(notation: &str) -> bool {
    let chiffres = notation.chars().take_while(|c| c.is_ascii_digit()).count();
    chiffres == 1 && notation.len() > 1
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
    fn trop_lent() {
        let mut t = Trial::new("test", parser("4HP, 4MK").unwrap());
        t.coup(&c("4HP"), 10);
        assert!(!t.temps_depasse(50));
        assert!(t.temps_depasse(200));
        assert_eq!(t.index, 0);
    }
}
