// Le serveur des clips partagés du SF6 Trainer (Worker Cloudflare).
//
// Il sert de gardien entre l'app et le stockage des vidéos (Backblaze B2, ou R2 plus tard) :
// l'app ne connaît jamais la clé du stockage, elle demande ici l'autorisation d'envoyer un
// clip, et c'est ici qu'on vérifie les limites (taille, envois par jour, place totale).
//
//   POST   /clips              autorisation d'envoyer un clip → { id, jeton, envoi }
//   POST   /clips/:id/fini     le clip est envoyé : vérifié puis publié
//   GET    /clips?personnage=&combo=   la liste des clips publiés
//   GET    /clips/:id.mp4      la vidéo (passe par ici, avec le cache de Cloudflare)
//   DELETE /clips/:id          supprimer (jeton de l'auteur, ou mot de passe admin)

import { AwsClient } from "aws4fetch";

const JOUR = 24 * 3600 * 1000;
/// Temps laissé pour envoyer le clip une fois l'autorisation donnée.
const DUREE_ENVOI_S = 15 * 60;

export default {
  async fetch(requete, env, ctx) {
    try {
      return await router(requete, env, ctx);
    } catch (e) {
      if (e instanceof Refus) {
        return json({ erreur: e.message }, e.statut);
      }
      console.error(e);
      return json({ erreur: "erreur du serveur" }, 500);
    }
  },
};

/// Une demande refusée, avec le message montré à l'utilisateur.
export class Refus extends Error {
  constructor(statut, message) {
    super(message);
    this.statut = statut;
  }
}

async function router(requete, env, ctx) {
  const url = new URL(requete.url);
  const morceaux = url.pathname.split("/").filter(Boolean);
  const methode = requete.method;

  if (morceaux[0] !== "clips") {
    throw new Refus(404, "adresse inconnue");
  }
  if (morceaux.length === 1 && methode === "POST") {
    return demanderEnvoi(requete, env);
  }
  if (morceaux.length === 1 && methode === "GET") {
    return lister(url, env);
  }
  if (morceaux.length === 3 && morceaux[2] === "fini" && methode === "POST") {
    return finirEnvoi(morceaux[1], env);
  }
  if (morceaux.length === 2 && morceaux[1].endsWith(".mp4") && methode === "GET") {
    return video(requete, morceaux[1].slice(0, -4), env, ctx);
  }
  if (morceaux.length === 2 && methode === "DELETE") {
    return supprimer(requete, morceaux[1], env);
  }
  throw new Refus(404, "adresse inconnue");
}

// ---------------------------------------------------------------------------------------
// Envoi d'un clip

async function demanderEnvoi(requete, env) {
  let corps;
  try {
    corps = await requete.json();
  } catch {
    throw new Refus(400, "demande illisible");
  }
  const pseudo = texte(corps.pseudo, 2, 24, "pseudo");
  const personnage = texte(corps.personnage, 1, 40, "personnage");
  const combo = texte(corps.combo, 1, 400, "combo");
  const taille = Number(corps.taille);
  const tailleMax = Number(env.TAILLE_MAX_OCTETS);
  if (!Number.isInteger(taille) || taille <= 0) {
    throw new Refus(400, "taille du clip manquante");
  }
  if (taille > tailleMax) {
    throw new Refus(413, `clip trop gros (${mo(taille)}, ${mo(tailleMax)} au plus)`);
  }

  const maintenant = Date.now();
  const ip = await empreinte(
    (requete.headers.get("CF-Connecting-IP") || "inconnue") + (env.ADMIN || ""),
  );
  await nettoyer(env, maintenant);

  const { n } = await env.DB.prepare(
    "SELECT COUNT(*) AS n FROM clips WHERE ip = ?1 AND cree_le > ?2",
  )
    .bind(ip, maintenant - JOUR)
    .first();
  if (n >= Number(env.ENVOIS_PAR_JOUR)) {
    throw new Refus(429, `${env.ENVOIS_PAR_JOUR} clips par jour au plus, réessaie demain`);
  }
  // La place prise : les clips publiés, et ceux en cours d'envoi.
  const { total } = await env.DB.prepare(
    "SELECT COALESCE(SUM(taille), 0) AS total FROM clips WHERE statut = 'publie' OR cree_le > ?1",
  )
    .bind(maintenant - JOUR)
    .first();
  if (total + taille > Number(env.TOTAL_MAX_OCTETS)) {
    throw new Refus(507, "plus de place pour de nouveaux clips pour l'instant");
  }

  const id = crypto.randomUUID();
  const jeton = crypto.randomUUID();
  const cle = `clips/${id}.mp4`;
  await env.DB.prepare(
    "INSERT INTO clips (id, cle, pseudo, personnage, combo, taille, statut, jeton, cree_le, ip)" +
      " VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'attente', ?7, ?8, ?9)",
  )
    .bind(id, cle, pseudo, personnage, combo, taille, await empreinte(jeton), maintenant, ip)
    .run();

  // Adresse signée : permet un seul envoi, de cette taille exacte, pendant 15 minutes.
  const adresse = new URL(adresseObjet(env, cle));
  adresse.searchParams.set("X-Amz-Expires", String(DUREE_ENVOI_S));
  const signee = await stockage(env).sign(
    new Request(adresse, {
      method: "PUT",
      headers: { "Content-Length": String(taille), "Content-Type": "video/mp4" },
    }),
    { aws: { signQuery: true, allHeaders: true } },
  );
  return json(
    {
      id,
      jeton,
      envoi: {
        methode: "PUT",
        adresse: signee.url,
        entetes: { "Content-Length": String(taille), "Content-Type": "video/mp4" },
      },
    },
    201,
  );
}

async function finirEnvoi(id, env) {
  const clip = await env.DB.prepare("SELECT * FROM clips WHERE id = ?1").bind(id).first();
  if (!clip) {
    throw new Refus(404, "clip inconnu");
  }
  if (clip.statut === "publie") {
    return json(public_(clip));
  }
  const reponse = await stockage(env).fetch(adresseObjet(env, clip.cle), { method: "HEAD" });
  if (!reponse.ok) {
    throw new Refus(409, "le clip n'a pas été reçu");
  }
  const recu = Number(reponse.headers.get("Content-Length"));
  if (recu !== clip.taille) {
    // Pas la taille annoncée : on ne garde rien.
    await effacer(env, clip);
    throw new Refus(409, "le clip reçu n'a pas la taille annoncée");
  }
  await env.DB.prepare("UPDATE clips SET statut = 'publie' WHERE id = ?1").bind(id).run();
  clip.statut = "publie";
  return json(public_(clip));
}

// ---------------------------------------------------------------------------------------
// Lecture

async function lister(url, env) {
  const personnage = url.searchParams.get("personnage");
  const combo = url.searchParams.get("combo");
  let requete = "SELECT * FROM clips WHERE statut = 'publie'";
  const valeurs = [];
  if (personnage) {
    valeurs.push(personnage);
    requete += ` AND personnage = ?${valeurs.length}`;
  }
  if (combo) {
    valeurs.push(combo);
    requete += ` AND combo = ?${valeurs.length}`;
  }
  requete += " ORDER BY cree_le DESC LIMIT 100";
  const { results } = await env.DB.prepare(requete)
    .bind(...valeurs)
    .all();
  return json({ clips: results.map(public_) });
}

async function video(requete, id, env, ctx) {
  const clip = await env.DB.prepare("SELECT * FROM clips WHERE id = ?1 AND statut = 'publie'")
    .bind(id)
    .first();
  if (!clip) {
    throw new Refus(404, "clip inconnu");
  }
  // Le cache de Cloudflare (seulement avec un nom de domaine à soi) : un clip regardé
  // souvent n'est lu qu'une fois dans le stockage, ce qui préserve son quota gratuit.
  const cache = typeof caches !== "undefined" ? caches.default : null;
  const cle = new Request(new URL(requete.url).toString(), { headers: requete.headers });
  if (cache) {
    const enCache = await cache.match(cle);
    if (enCache) {
      return enCache;
    }
  }
  const reponse = await stockage(env).fetch(adresseObjet(env, clip.cle));
  if (!reponse.ok) {
    throw new Refus(502, "vidéo introuvable dans le stockage");
  }
  const entetes = new Headers({
    "Content-Type": "video/mp4",
    "Cache-Control": "public, max-age=31536000, immutable",
    "Accept-Ranges": "bytes",
  });
  const longueur = reponse.headers.get("Content-Length");
  if (longueur) {
    entetes.set("Content-Length", longueur);
  }
  const complete = new Response(reponse.body, { status: 200, headers: entetes });
  if (cache) {
    // Le cache rend lui-même les morceaux demandés (lecture à partir d'un moment précis).
    ctx.waitUntil(cache.put(new Request(cle.url), complete.clone()));
  }
  return complete;
}

// ---------------------------------------------------------------------------------------
// Suppression

async function supprimer(requete, id, env) {
  const clip = await env.DB.prepare("SELECT * FROM clips WHERE id = ?1").bind(id).first();
  if (!clip) {
    throw new Refus(404, "clip inconnu");
  }
  const admin = env.ADMIN && requete.headers.get("Authorization") === `Bearer ${env.ADMIN}`;
  const jeton = requete.headers.get("X-Jeton");
  const auteur = jeton && (await empreinte(jeton)) === clip.jeton;
  if (!admin && !auteur) {
    throw new Refus(403, "seul l'auteur du clip (ou l'admin) peut le supprimer");
  }
  await effacer(env, clip);
  return json({ supprime: id });
}

async function effacer(env, clip) {
  await stockage(env).fetch(adresseObjet(env, clip.cle), { method: "DELETE" });
  await env.DB.prepare("DELETE FROM clips WHERE id = ?1").bind(clip.id).run();
}

/// Les envois autorisés mais jamais terminés (plus d'un jour) sont oubliés.
async function nettoyer(env, maintenant) {
  const { results } = await env.DB.prepare(
    "SELECT * FROM clips WHERE statut = 'attente' AND cree_le < ?1 LIMIT 20",
  )
    .bind(maintenant - JOUR)
    .all();
  for (const clip of results) {
    await effacer(env, clip);
  }
}

// ---------------------------------------------------------------------------------------
// Outils

function stockage(env) {
  return new AwsClient({
    accessKeyId: env.STOCKAGE_CLE_ID,
    secretAccessKey: env.STOCKAGE_CLE,
    service: "s3",
    region: env.STOCKAGE_REGION,
  });
}

function adresseObjet(env, cle) {
  return `${env.STOCKAGE_ADRESSE.replace(/\/$/, "")}/${env.STOCKAGE_BUCKET}/${cle}`;
}

/// Ce que tout le monde peut voir d'un clip (ni l'IP, ni le jeton).
function public_(clip) {
  return {
    id: clip.id,
    pseudo: clip.pseudo,
    personnage: clip.personnage,
    combo: clip.combo,
    taille: clip.taille,
    date: new Date(clip.cree_le).toISOString(),
    video: `/clips/${clip.id}.mp4`,
  };
}

function texte(valeur, min, max, nom) {
  if (typeof valeur !== "string") {
    throw new Refus(400, `${nom} manquant`);
  }
  const t = valeur.trim();
  if ([...t].length < min || [...t].length > max) {
    throw new Refus(400, `${nom} : entre ${min} et ${max} caractères`);
  }
  if (/[\u0000-\u001f\u007f]/.test(t)) {
    throw new Refus(400, `${nom} : caractères interdits`);
  }
  return t;
}

async function empreinte(texte) {
  const octets = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(texte));
  return [...new Uint8Array(octets)].map((o) => o.toString(16).padStart(2, "0")).join("");
}

function mo(octets) {
  return `${(octets / 1e6).toFixed(1)} Mo`;
}

function json(valeur, statut = 200) {
  return new Response(JSON.stringify(valeur), {
    status: statut,
    headers: { "Content-Type": "application/json; charset=utf-8" },
  });
}
