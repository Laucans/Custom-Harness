// What the mock server does with a DataCapability's command: one function
// per identifier, `(server, { payload, targetId, version, basedOn }) =>
// violations | 'not-found' | undefined`. It judges the payload as the
// aggregate's invariants would (answer the violation ids), changes the
// fixtures it loaded (`server.load(name)`), and answers nothing when
// applied; the server bumps `dataVersion` and every micro-frontend
// refetches. A DataCapability absent here is applied as is.
//
//   'campagne.creerCampagne': async (server, { payload }) => {
//     if (!String(payload.name || '').trim()) return ['campaign-name-required'];
//     (await server.load('campagnes')).push({ id: server.uuid(), name: payload.name });
//   },

export const COMMANDS = {};
