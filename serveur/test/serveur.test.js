// Test de bout en bout : le Worker tourne en local (`wrangler dev`, avec une vraie base D1
// locale), et un faux stockage S3 remplace B2. Lancer avec `npm test`.

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { spawn, execFileSync } from "node:child_process";
import { writeFileSync, rmSync } from "node:fs";

const PORT_STOCKAGE = 18_999;
const PORT_WORKER = 18_787;
const SERVEUR = `http://127.0.0.1:${PORT_WORKER}`;
const dossier = new URL("..", import.meta.url).pathname;

/// Le faux stockage : garde les objets en mémoire.
const objets = new Map();
let stockageHttp;
let worker;

before(async () => {
  stockageHttp = http.createServer((req, res) => {
    const cle = req.url.split("?")[0];
    if (req.method === "PUT") {
      const morceaux = [];
      req.on("data", (m) => morceaux.push(m));
      req.on("end", () => {
        objets.set(cle, Buffer.concat(morceaux));
        res.end();
      });
      return;
    }
    const objet = objets.get(cle);
    if (req.method === "DELETE") {
      objets.delete(cle);
      res.statusCode = 204;
      return res.end();
    }
    if (!objet) {
      res.statusCode = 404;
      return res.end();
    }
    res.setHeader("Content-Length", objet.length);
    res.end(req.method === "HEAD" ? undefined : objet);
  });
  await new Promise((ok) => stockageHttp.listen(PORT_STOCKAGE, "127.0.0.1", ok));

  rmSync(`${dossier}.wrangler`, { recursive: true, force: true });
  writeFileSync(
    `${dossier}.dev.vars`,
    [
      `STOCKAGE_ADRESSE=http://127.0.0.1:${PORT_STOCKAGE}`,
      "STOCKAGE_CLE_ID=essai",
      "STOCKAGE_CLE=essai",
      "ADMIN=motdepasse",
      "ENVOIS_PAR_JOUR=3",
      "TAILLE_MAX_OCTETS=1000",
    ].join("\n"),
  );
  execFileSync(
    "npx",
    ["wrangler", "d1", "execute", "sf6-trainer-clips", "--local", "--file", "schema.sql"],
    { cwd: dossier, stdio: "ignore" },
  );
  worker = spawn(
    "npx",
    ["wrangler", "dev", "--local", "--port", String(PORT_WORKER), "--ip", "127.0.0.1"],
    { cwd: dossier, stdio: "ignore" },
  );
  for (let i = 0; i < 120; i++) {
    try {
      await fetch(`${SERVEUR}/clips`);
      return;
    } catch {
      await new Promise((ok) => setTimeout(ok, 500));
    }
  }
  throw new Error("le Worker ne démarre pas");
});

after(() => {
  worker?.kill();
  stockageHttp?.close();
  rmSync(`${dossier}.dev.vars`, { force: true });
});

async function demander(corps) {
  const r = await fetch(`${SERVEUR}/clips`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(corps),
  });
  return { statut: r.status, corps: await r.json() };
}

const CLIP = { pseudo: "Clément", personnage: "Dhalsim", combo: "[4HP] xx DR xx 4MK" };

test("un clip envoyé est publié, listé, lu, puis supprimé par son auteur", async () => {
  const video = Buffer.alloc(600, 7);
  const { statut, corps } = await demander({ ...CLIP, taille: video.length });
  assert.equal(statut, 201, JSON.stringify(corps));
  assert.match(corps.envoi.adresse, /X-Amz-Signature=/);

  // Pas encore envoyé : pas publiable.
  let r = await fetch(`${SERVEUR}/clips/${corps.id}/fini`, { method: "POST" });
  assert.equal(r.status, 409);

  r = await fetch(corps.envoi.adresse, {
    method: "PUT",
    headers: corps.envoi.entetes,
    body: video,
  });
  assert.equal(r.status, 200);
  r = await fetch(`${SERVEUR}/clips/${corps.id}/fini`, { method: "POST" });
  assert.equal(r.status, 200);

  const liste = await (await fetch(`${SERVEUR}/clips?personnage=Dhalsim`)).json();
  assert.equal(liste.clips.length, 1);
  assert.equal(liste.clips[0].pseudo, "Clément");
  assert.equal(liste.clips[0].ip, undefined);

  r = await fetch(`${SERVEUR}${liste.clips[0].video}`);
  assert.equal(r.status, 200);
  assert.deepEqual(Buffer.from(await r.arrayBuffer()), video);

  r = await fetch(`${SERVEUR}/clips/${corps.id}`, {
    method: "DELETE",
    headers: { "X-Jeton": "pas le bon" },
  });
  assert.equal(r.status, 403);
  r = await fetch(`${SERVEUR}/clips/${corps.id}`, {
    method: "DELETE",
    headers: { "X-Jeton": corps.jeton },
  });
  assert.equal(r.status, 200);
  assert.equal(objets.size, 0);
  const vide = await (await fetch(`${SERVEUR}/clips`)).json();
  assert.equal(vide.clips.length, 0);
});

test("les limites sont respectées", async () => {
  let r = await demander({ ...CLIP, taille: 5000 });
  assert.equal(r.statut, 413);
  r = await demander({ ...CLIP, pseudo: "x", taille: 10 });
  assert.equal(r.statut, 400);

  // Une taille différente de celle annoncée : rien n'est gardé.
  r = await demander({ ...CLIP, taille: 100 });
  assert.equal(r.statut, 201);
  await fetch(r.corps.envoi.adresse, { method: "PUT", body: Buffer.alloc(50) });
  const fini = await fetch(`${SERVEUR}/clips/${r.corps.id}/fini`, { method: "POST" });
  assert.equal(fini.status, 409);

  // 3 envois par jour (les clips supprimés ne comptent plus) : le 4e est refusé.
  for (let i = 0; i < 3; i++) {
    r = await demander({ ...CLIP, taille: 10 });
    assert.equal(r.statut, 201);
  }
  r = await demander({ ...CLIP, taille: 10 });
  assert.equal(r.statut, 429);
});

test("l'admin peut supprimer n'importe quel clip", async () => {
  const liste = await (await fetch(`${SERVEUR}/clips`)).json();
  assert.equal(liste.clips.length, 0);
  const r = await fetch(`${SERVEUR}/clips/inconnu`, {
    method: "DELETE",
    headers: { Authorization: "Bearer motdepasse" },
  });
  assert.equal(r.status, 404);
});
