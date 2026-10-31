-- Le catalogue des clips partagés. Les vidéos elles-mêmes sont dans le stockage (B2 ou R2),
-- sous le nom `cle` : le catalogue garde ce nom, jamais l'adresse complète, pour pouvoir
-- changer de stockage sans rien toucher ici.
CREATE TABLE IF NOT EXISTS clips (
  id TEXT PRIMARY KEY,
  cle TEXT NOT NULL,
  pseudo TEXT NOT NULL,
  personnage TEXT NOT NULL,
  combo TEXT NOT NULL,
  taille INTEGER NOT NULL,
  -- « attente » : autorisé mais pas encore envoyé ; « publie » : visible par tous.
  statut TEXT NOT NULL,
  -- Empreinte du jeton donné à l'auteur : il peut supprimer son clip avec.
  jeton TEXT NOT NULL,
  cree_le INTEGER NOT NULL,
  ip TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS clips_combo ON clips (personnage, combo, statut, cree_le);
CREATE INDEX IF NOT EXISTS clips_ip ON clips (ip, cree_le);
