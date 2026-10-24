# The mock server's handlers

One file per unit: `mock/handlers/<id>.js` teaches the mock server
(`mock/src/server.js`) what a Capability answers or what a DataCapability
judges, when a fixture as it is is not enough. The server imports the file
the first time `<id>` is read or commanded; it is never bundled, so a
handler changed rebuilds nothing (`mock/build.sh` is for `apps/` and
`mock/src/`).

```js
// mock/handlers/campagne.listerPjs.js — a Capability: `(server, variables) => data | undefined`
export const reads = {
  'campagne.listerPjs': async (server, { campagneId }) =>
    (await server.load('pjs')).filter((p) => p.campagneId === campagneId),
};
```

```js
// mock/handlers/campagne.creerCampagne.js — a DataCapability: `(server, { payload, targetId, version, basedOn }) => violations | undefined`
export const commands = {
  'campagne.creerCampagne': async (server, { payload }) => {
    if (!String(payload.name || '').trim()) return ['campaign-name-required'];
    (await server.load('campagnes')).push({ id: server.uuid(), name: payload.name });
  },
};
```

- A file defines its own id only, under `reads` or `commands`; the server
  answers a key of another id with an error, and so does the gate.
- `server.load(name)` is a mutable fixture (`mock/fixtures/<name>.json`),
  `server.uuid()` an id, `server.bump()` tells every micro-frontend the data
  changed. A read with no handler answers `mock/fixtures/<id>.json`; a
  command with no handler is **rejected** (`unhandled-data-capability`), as
  the real server rejects what no manifest registers.
- The browser loads the file as a module beside the page: its imports are
  relative. A helper several files share goes in `mock/handlers/_lib/`.
- Your unit's file is yours; another unit's is its task's — never edit it.
  Never add a handler to the legacy `mock/src/reads.js` or
  `mock/src/commands.js`: `node scripts/split-handlers.mjs` turns them into
  handler files, once per tree.
- An id defined twice — in its file and a legacy file, or in another id's
  file — is an error naming both files, never an override. The server finds
  it at startup for every id the legacy files or `mock/pages.json`
  (`needs`, `actions`) name, and then answers every call 500 with
  `server.problems`; any other id is checked on its first use.
- The gate `mock-handlers` (`scripts/mock-handlers-gate.mjs`) refuses an id
  defined twice and, once this folder holds a handler file, a handler added
  to a legacy file.
- Until the split, the legacy files still count in every bundle's
  `builtFrom` digest (as an older `mock/build.mjs` counts them): changing
  one means `sh mock/build.sh`. A handler file never does.
