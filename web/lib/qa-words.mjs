// What a QA presenter's call answers it, worded for the model: the same for the MCP tools
// (bin/qa-mcp.mjs, a new presenter) and the command (bin/qa.mjs, a session presenting its own work).
// `r` is lib/qa-live.mjs's result; `how` names the calls the way this presenter makes them.

/** @param {'mcp' | 'cli'} how */
export function words(name, r, how = 'mcp') {
  const listen = how === 'cli' ? '`Q listen`' : 'listen()';
  const pass = (id) => (how === 'cli' ? `--answering ${id}` : `answering: "${id}"`);
  if (r?.ended) {
    return 'The QA session is over. Stop presenting, and go back to what you were doing before it started.';
  }
  if (r?.interrupted) {
    return `The user interrupted you and asked: "${r.question}". Everything you had decided to say before this is dropped. Answer it now, passing ${pass(r.id)} in every say and show until you call ${listen}: acknowledge it in one short spoken sentence (say), look at the real code first if the question needs it (status while you do), show what you found, then say the answer, ask whether to go on, and call ${listen}.`;
  }
  if (r?.dropped) {
    return `Not said: the user interrupted you and asked: "${r.question}". Answer that first, passing ${pass(r.id)} in say and show, then call ${listen}.`;
  }
  if (name === 'listen') {
    if (r?.continue) return 'The user wants you to go on: continue the talk where it fits, with show and say.';
    return `Nothing yet. Call ${listen} again.`;
  }
  const note = how === 'cli' ? r?.note?.replaceAll('listen()', listen) : r?.note;
  return note ? `ok (${note})` : 'ok';
}
