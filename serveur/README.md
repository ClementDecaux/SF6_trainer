# Serveur des clips partagés

Le petit serveur qui permet à n'importe quel utilisateur du trainer de partager ses clips.
Il tourne gratuitement chez Cloudflare (un « Worker » et une base « D1 »). Les vidéos sont
rangées chez Backblaze B2 (10 Go gratuits), avec des plafonds à 0 € : B2 refuse au lieu
de facturer.

L'app ne parle qu'à ce serveur. Elle ne connaît jamais la clé du stockage. Le serveur
vérifie les limites avant chaque envoi :

- un pseudo de 2 à 24 caractères ;
- 15 Mo au plus par clip (`TAILLE_MAX_OCTETS`) ;
- 20 envois par jour et par connexion (`ENVOIS_PAR_JOUR`) ;
- 9 Go au total (`TOTAL_MAX_OCTETS`), sous les 10 Go gratuits de B2.

Chaque auteur peut retirer ses clips depuis l'app. L'admin peut supprimer n'importe quel
clip (voir plus bas).

## Mise en ligne (une seule fois, environ 20 minutes)

### 1. Backblaze B2 : le stockage

1. Crée un compte sur [backblaze.com](https://www.backblaze.com/sign-up/cloud-storage).
2. **Caps & Alerts** : mets chaque plafond (stockage, Class A, B et C) à **0 $** par jour.
   Une fois l'offre gratuite épuisée, B2 refuse les envois ou les lectures au lieu de
   facturer.
3. **Buckets → Create a Bucket** : un nom unique, par exemple `sf6-trainer-clips-clement`,
   **Private**. Sur la page du bucket, note l'**Endpoint**, par exemple
   `s3.eu-central-003.backblazeb2.com`.
4. **Application Keys → Add a New Application Key** : accès à ce bucket seulement, en
   *Read and Write*. Note le **keyID** et l'**applicationKey** : la clé n'est montrée
   qu'une fois.

### 2. Cloudflare : le serveur

Il faut [Node.js](https://nodejs.org) (version LTS) et un compte Cloudflare gratuit.
Dans ce dossier `serveur/` :

```
npm install
npx wrangler login
npx wrangler d1 create sf6-trainer-clips
```

La dernière commande affiche un `database_id` : copie-le dans `wrangler.toml`. Dans le
même fichier, remplis `[vars]` avec ton endpoint B2 (précédé de `https://`), la région
(la partie `eu-central-003` de l'endpoint) et le nom du bucket. Ensuite :

```
npx wrangler d1 execute sf6-trainer-clips --remote --file schema.sql
npx wrangler secret put STOCKAGE_CLE_ID
npx wrangler secret put STOCKAGE_CLE
npx wrangler secret put ADMIN
npx wrangler deploy
```

Les trois `secret put` demandent la valeur : le keyID de B2, l'applicationKey de B2, puis
un mot de passe admin de ton choix. `deploy` affiche l'adresse du serveur, par exemple
`https://sf6-trainer-clips.ton-nom.workers.dev`.

### 3. Dans le trainer

Dans **Réglages → Partage des clips en ligne**, colle l'adresse du serveur, choisis un
pseudo, puis clique sur Enregistrer. Le bouton **Partager en ligne** apparaît sous chaque
vidéo.

## Administration

Lister les clips :

```
curl https://<serveur>/clips
```

Supprimer un clip :

```
curl -X DELETE -H "Authorization: Bearer <mot de passe admin>" https://<serveur>/clips/<id>
```

## Plus tard

- **Cache** : avec un nom de domaine à toi branché sur Cloudflare, les vidéos regardées
  souvent sont servies depuis le cache de Cloudflare. B2 n'est alors lu qu'une fois par
  clip, ce qui préserve son quota gratuit de lectures.
- **Passer à Cloudflare R2** : copie le bucket avec l'outil de migration de R2 (Super
  Slurper), puis change `STOCKAGE_ADRESSE`, `STOCKAGE_REGION` (`auto`) et les deux clés.
  L'app et le catalogue ne changent pas.

## Tests

`npm test` lance le serveur en local avec un faux stockage, et vérifie l'envoi, la
publication, la lecture, la suppression et les limites.
