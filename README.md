# SF6 Combos Trainer

Overlay de trials pour Street Fighter 6, en Rust. Il s'affiche par-dessus le jeu, passe à l'étape
suivante quand tu fais le bon coup, et montre ton historique d'inputs à la frame près.
Chaque combo réussi joue un petit son et est enregistré en vidéo MP4 (60 fps) dans `clips/`.

L'outil ne touche jamais au jeu : il lit seulement ta manette (aucun risque avec l'anti-cheat).

## Lancer

Windows, avec Rust installé ([rustup.rs](https://rustup.rs)) :

```powershell
cargo run          # lance le 1er combo de combos.toml
cargo run -- 2     # lance le 2e
cargo test         # lance les tests
```

- SF6 doit être en **plein écran fenêtré** (borderless), sinon rien ne peut s'afficher par-dessus.
- Le contrôleur doit être en mode **XInput** (mode manette Xbox).
- Fermer la fenêtre de l'interface (ou Ctrl+C dans le terminal) ferme l'outil.
- Pour **déplacer** l'overlay, glisse la petite bande en haut. **F8** active en plus le
  redimensionnement par les bords et les coins (F8 à nouveau pour finir). La place est
  enregistrée dans `overlay.toml`.
- La **languette** en bas de l'overlay déplie ou replie l'historique d'inputs.
- **F9** pour garder ou non les vidéos des combos réussis (`REC` dans l'overlay). Coupé au
  lancement, sauf si `enregistrer = true` dans `capture.toml`.
- Côté de l'écran : par défaut l'outil le reconnaît au 1er coup de chaque essai (un combo joué
  à droite est retourné : 214 à l'écran = 236). On peut le forcer dans l'onglet Entraînement.

## L'interface

Au lancement, une fenêtre normale s'ouvre en plus de l'overlay. Deux onglets :

- **Combos** : la liste rangée par catégorie, avec une recherche. Clique un combo pour voir
  ses étapes, ses hits et dégâts, puis **Travailler ce combo** pour le passer dans l'overlay.
  En dessous, ses vidéos : la plus récente s'ouvre d'office, les autres se choisissent dans la
  liste. Le lecteur avance image par image (← →, ou les boutons), lit au ralenti (x0.5, x0.25,
  x0.1), et place chaque étape du combo sur la barre de temps : cliquer une étape y amène. À
  droite, l'historique du jeu tel qu'il était à cette image. **Plein écran** agrandit le
  lecteur (Échap pour sortir).
- **Entraînement** : le côté de l'écran, et l'**auto-advance** : coche les combos qui
  t'intéressent (par catégorie ou un par un), choisis après combien de réussites (d'affilée ou
  au total) on passe au suivant, dans l'ordre ou au hasard. L'overlay affiche la progression
  (« auto 2/3 d'affilée »). Réglages gardés dans `entrainement.toml`.
- **Réglages** : afficher ou cacher l'overlay, le déplacer, son opacité, le son, les vidéos,
  et la façon d'écrire les coups.

### Écriture des coups

Quatre syntaxes au choix (Réglages, ou `syntaxe = "..."` dans `overlay.toml`) :
`sf6` (flèches et pastilles poing / pied colorées comme dans le jeu, par défaut), `numpad`
(`236HP`), `anglais` (`qcf+HP`, `cr.MK`) ou `fleches` (flèches puis `HP`). Tout suit ce choix :
étapes et historique de l'overlay, message de raté, liste des combos et lecteur vidéo. Le
terminal reste en numpad. Dans `combos.toml`, on peut écrire en numpad, en anglais ou avec les
flèches `↓↘→`.

Les liaisons entre les coups (cancel, link) ont aussi leur style, dans les mêmes réglages
(`liaisons = "..."` dans `overlay.toml`) : `pastilles` (étiquettes CANCEL orange et LINK bleue,
par défaut), `mots`, `symboles` (`xx` et `,`), `courts` (`>` et `,`) ou `aucune`. Dans
l'overlay, elles ont une colonne de largeur fixe : les coups sont alignés.

Le bouton en haut à droite cache ou réaffiche l'overlay. L'overlay n'apparaît que quand SF6
est la fenêtre active (sauf en mode déplacement F8) ; décochable dans les réglages
(`seulement_jeu` dans `overlay.toml`). Fermer l'interface ferme l'outil.

## Fichiers de config

| Fichier | Contenu |
| --- | --- |
| `combos.toml` | Tes combos, en notation forum : `b+hp xx DR xx b+mk xx qcf+mk, b+mk xx hcb+mp` |
| `controles.toml` | Quel bouton physique fait quoi dans SF6 (LP, MP, HP...) |
| `overlay.toml` | Position, taille et opacité de l'overlay, son du combo réussi |
| `timings.toml` | Hits (et écarts) appris sur tes vrais combos (créé au premier Select) |
| `capture.toml` | Vidéo des combos réussis : on/off, dossier, durée filmée avant et après le combo, qualité |

## Choisir le combo

Les combos sont dans `combos.toml` (une liste de combos Dhalsim est créée au premier lancement).
Le plus simple : l'onglet **Combos** de l'interface. Les flèches ‹ › de l'overlay passent
aussi au combo précédent ou suivant ; `cargo run -- 5` lance
directement le 5e. La liste numérotée s'affiche dans le terminal au lancement.

## Validation d'un combo

Une étape avance quand le bon coup arrive, quel que soit le moment (il n'y a plus de fenêtres
de timing : elles se trompaient trop souvent). Un autre coup est un raté ; un ré-appui du
bouton de l'étape d'avant (double tap, mash) est ignoré.

C'est le panneau des dégâts du jeu (voir plus bas) qui dit si le combo est passé : tous les
coups attendus doivent avoir touché. L'écart en frames entre chaque étape et la précédente
reste affiché (`+14 f`), pour information.

### Coups lus dans le jeu

Quand l'historique d'inputs du jeu est visible (voir ci-dessous), les coups du trial sont lus
dans ses lignes : boutons appuyés, et mouvements (236, 63214...) reconnus dans les directions
affichées par le jeu. C'est donc ce que le jeu a vraiment pris en compte, avec quelques frames
de retard (le son d'étape arrive un peu après l'appui). Sans historique visible, l'outil lit
la manette.

### Frames exactes : l'historique d'inputs du jeu

Active l'affichage des inputs dans le mode entraînement de SF6. L'outil lit alors l'historique
à gauche de l'écran sur les images capturées (chiffres, flèches, boutons, comparés à des modèles
pris sur des photos du jeu). Quand un combo est fini, il retrouve chaque étape dans cet
historique : les écarts sont ceux du jeu, à la frame près (`frames exactes du jeu` dans le
terminal). Le verdict arrive donc environ 0,1 s après le dernier coup.

Si l'historique n'est pas visible ou mal lu (autre affichage par-dessus, stage trop clair...),
l'outil revient à son estimation (`frames estimées`, à une frame près).
`historique = false` dans `capture.toml` coupe cette lecture.

### Coups qui touchent : le panneau des dégâts

Affiche les infos de dégâts du mode entraînement (« Damage / Combo Damage », en haut au milieu).
L'outil voit les dégâts du combo monter à chaque coup qui touche. Quand tu confirmes un vrai
combo avec Select, il retient son nombre de hits (dans `timings.toml`). Ensuite, un combo est
validé si tous ses coups ont touché, quel que soit le timing ; sinon : « 4/7 hits : lâché
après 236MK ».

Les dégâts du combo complet (`degats` dans `combos.toml`, ou appris avec Select) servent aussi :
le combo est validé si les hits **ou** les dégâts sont atteints. Deux coups qui touchent dans
la même image ne comptent que pour un hit, mais les dégâts, eux, sont justes.

## Vidéos des combos réussis

Rien à installer (pas d'OBS) : l'outil filme la fenêtre de SF6 avec l'API de capture de Windows.
Il garde toujours en mémoire les dernières images du jeu (`avant_ms`, 0,7 s par défaut) : la vidéo
commence donc un peu avant la 1re étape. Elle est jetée si le combo est raté, et gardée dans
`clips/` s'il est réussi. Un seul exemplaire par combo : la dernière réussite remplace la
vidéo précédente (`Dhalsim_Drive-Rush-Yoga-Flame.mp4`). L'overlay n'apparaît pas dans la vidéo.

À côté de chaque vidéo, un `.json` donne le combo, le personnage, la date, l'heure de chaque
étape et l'historique du jeu pendant la vidéo : c'est ce qui permet au lecteur de l'interface
(onglet Vidéos) de placer les étapes à la bonne image. Les vidéos sont en H.264 à 60 fps ; le
lecteur les décode avec Windows (Media Foundation), en 960 pixels de large au plus.

Notation reconnue : directions `b f d u db df cr. st.` ou chiffres numpad, mouvements
`qcf qcb hcf hcb dp` ou `236 214 41236 63214 623 421`, `DI` (HP+HK), `DP` (MP+MK),
`DR` (66, ou MP+MK pendant un coup annulable), `xx` ou `>` pour un cancel, `,` pour un link.

## Organisation du code

| Fichier | Rôle |
| --- | --- |
| `src/main.rs` | Lancement : charge les configs, ouvre l'interface, démarre le thread de la manette |
| `src/interface.rs` | Fenêtre principale : combos, lecteur vidéo, réglages ; ouvre l'overlay |
| `src/syntaxe.rs` | Les syntaxes des coups (numpad, anglais, flèches, icônes SF6) et leur dessin |
| `src/entrainement.rs` | Auto-advance : sélection de combos et passage au suivant |
| `src/video.rs` | Décodage des vidéos MP4 image par image (Media Foundation) |
| `src/entrees.rs` | Thread de la manette : lit les inputs, reconnaît les coups, fait avancer le trial |
| `src/manette.rs` | Lecture de la manette : XInput toutes les millisecondes sous Windows |
| `src/ecran.rs` | Lecture de l'historique d'inputs affiché par SF6 (frames exactes) |
| `src/ecran_modeles.rs` | Modèles des chiffres, flèches et boutons de SF6 (fichier généré) |
| `src/horloge.rs` | Numéros de frame calés sur les images de SF6 (reçues par la capture) |
| `src/notation.rs` | Directions -> notation numpad (1 à 9) |
| `src/motion.rs` | Reconnaissance des mouvements spéciaux (236, 63214, 66...) |
| `src/combo.rs` | Lecture des combos écrits en notation forum, moteur de trial |
| `src/controles.rs` | Correspondance boutons physiques -> actions SF6 |
| `src/partage.rs` | État partagé entre le thread de la manette et l'overlay |
| `src/overlay.rs` | Fenêtre de l'overlay (egui) |
| `src/affichage.rs` | Historique d'inputs coloré dans le terminal |
| `src/capture.rs` | Vidéo des combos réussis (thread de capture Windows) |
| `src/timings.rs` | Hits appris sur tes vrais combos |
| `src/clavier.rs` | Touche F8 (déplacer l'overlay), lue même quand le jeu a le focus |
| `src/son.rs` | Petit son du combo réussi, fabriqué par le programme |

## Apprendre Rust avec ce projet

- [Le Rust Book](https://doc.rust-lang.org/book/) ([en français](https://jimskapt.github.io/rust-book-fr/)) et [Rustlings](https://github.com/rust-lang/rustlings).
- `struct`, `enum`, `match` : `notation.rs`, `motion.rs`
- Erreurs avec `Result` : `combo.rs` (`parser`, `normaliser`)
- Tests : les blocs `#[cfg(test)]` en bas de chaque fichier
- Threads et `Arc<Mutex<_>>` : `main.rs`, `partage.rs`
- Canal entre threads (`mpsc`) et code pour Windows seulement (`#[cfg(windows)]`) : `capture.rs`

Icônes : Material Design Icons (Pictogrammers, licence Apache 2.0), via Iconify.
