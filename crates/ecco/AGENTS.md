# ecco

- Keep declarations and extracted graphs owned; never retain OXC arena data.
- Value IDs are context-local identities. Passthrough returns reuse them.
- Hazards only grow within a solve; record dependencies even on false facts.
- Evaluate against immutable rounds and merge writes before scheduling readers.
- Binding matching uses OXC semantics, never identifier spelling alone.
- Record function operations without executing them; rollback recording-only
  graph allocations and instantiate owned summaries at calls.
- Lexical binding slots are not value identities. Rebinding must not redirect
  existing aliases; each invocation owns its locals and closure captures.
- Keep unspecialized callable hazards separate from call-target mutation markers.
- Callback effect queries are latent; only execution applies external updates.
- Unknown behavior must not become a proof of purity. Exceptions alone are not
  retention effects. This does not permit reordering or duplicating expressions.
