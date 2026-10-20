# ecco

- Keep declarations and extracted graphs owned; never retain OXC arena data.
- Value IDs are context-local identities. Passthrough returns reuse them.
- Hazards only grow within a solve; record dependencies even on false facts.
- Evaluate against immutable rounds and merge writes before scheduling readers.
- Binding matching uses OXC semantics, never identifier spelling alone.
- Unknown behavior must not become a proof of purity. Exceptions alone are not
  retention effects. This does not permit reordering or duplicating expressions.
