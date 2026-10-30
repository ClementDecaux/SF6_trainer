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
- Ctrl+C dans le terminal pour fermer l'overlay.
- **F8** pour déplacer l'overlay : glisse-le à la souris, puis F8 à nouveau. La position est
  enregistrée dans `overlay.toml`. Reclique ensuite dans le jeu pour lui rendre le focus.
- **F9** pour garder ou non les vidéos des combos réussis (`● REC` dans l'overlay). Coupé au
  lancement, sauf si `enregistrer = true` dans `capture.toml`.
- L'outil suppose que tu joues à gauche de l'écran (b = 4 = vers la gauche).

## Fichiers de config

| Fichier | Contenu |
| --- | --- |
| `combos.toml` | Tes combos, en notation forum : `b+hp xx DR xx b+mk xx qcf+mk, b+mk xx hcb+mp` |
| `controles.toml` | Quel bouton physique fait quoi dans SF6 (LP, MP, HP...) |
| `overlay.toml` | Position, taille et opacité de l'overlay, son du combo réussi |
| `timings.toml` | Fenêtres de timing apprises (créé au premier Select) |
| `capture.toml` | Vidéo des combos réussis : on/off, dossier, durée filmée avant et après le combo, qualité |

## Timing des étapes

L'outil lit seulement la manette : il ne voit pas si un coup touche. Il mesure donc l'écart en
frames entre chaque étape et la précédente (affiché `+14 f` dans l'overlay et le terminal).

Quand tu vois dans le jeu qu'un combo est vraiment passé, appuie sur **Select** dans les
10 secondes : ses écarts sont ajoutés à `timings.toml`. Après 3 combos confirmés,
l'outil connaît la fenêtre de chaque étape (1 frame de tolérance) :
- avant la fenêtre, les appuis sont ignorés, comme dans le jeu (double tap, coup intercalé) ;
- dans la fenêtre, le bon coup valide l'étape, un autre coup est un raté ;
- après la fenêtre : « 4MK trop tard de 3 f », pas de son et pas de vidéo.

Si un vrai combo est marqué « trop tard », appuie quand même sur Select : la fenêtre s'élargit.

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
après 236MK ». C'est plus fiable que les fenêtres de timing (positionnement, garde...).

## Vidéos des combos réussis

Rien à installer (pas d'OBS) : l'outil filme la fenêtre de SF6 avec l'API de capture de Windows.
Il garde toujours en mémoire les dernières images du jeu (`avant_ms`, 0,7 s par défaut) : la vidéo
commence donc un peu avant la 1re étape. Elle est jetée si le combo est raté, et gardée dans
`clips/` s'il est réussi. L'overlay n'apparaît pas dans la vidéo.

À côté de chaque vidéo, un `.json` donne le combo, le personnage, la date et la frame de chaque
étape (comptée depuis la 1re étape). Les vidéos sont en H.264 à 60 fps : on peut les avancer image
par image, par exemple dans VLC (touche `E`) ou mpv (touche `.`).

Notation reconnue : directions `b f d u db df cr. st.` ou chiffres numpad, mouvements
`qcf qcb hcf hcb dp` ou `236 214 41236 63214 623 421`, `DI` (HP+HK), `DP` (MP+MK),
`DR` (66, ou MP+MK pendant un coup annulable), `xx` ou `>` pour un cancel, `,` pour un link.

## Organisation du code

| Fichier | Rôle |
| --- | --- |
| `src/main.rs` | Lancement : charge les configs, ouvre l'overlay, démarre le thread de la manette |
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
| `src/timings.rs` | Fenêtres de timing apprises sur tes vrais combos |
| `src/clavier.rs` | Touche F8 (déplacer l'overlay), lue même quand le jeu a le focus |
| `src/son.rs` | Petit son du combo réussi, fabriqué par le programme |

## Apprendre Rust avec ce projet

- [Le Rust Book](https://doc.rust-lang.org/book/) ([en français](https://jimskapt.github.io/rust-book-fr/)) et [Rustlings](https://github.com/rust-lang/rustlings).
- `struct`, `enum`, `match` : `notation.rs`, `motion.rs`
- Erreurs avec `Result` : `combo.rs` (`parser`, `normaliser`)
- Tests : les blocs `#[cfg(test)]` en bas de chaque fichier
- Threads et `Arc<Mutex<_>>` : `main.rs`, `partage.rs`
- Canal entre threads (`mpsc`) et code pour Windows seulement (`#[cfg(windows)]`) : `capture.rs`
