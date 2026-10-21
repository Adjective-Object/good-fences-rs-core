# ecco

- Read persisted precursors and nested contracts through borrowed accessors.
  Keep extraction construction distinct from immutable parsed files and mutable
  interpreter state; retain Serde names/order when moving record definitions.
- `views` returns Copy owned-or-wire records and generated-vector iterators;
  never unpack a cache tree. Analysis retains `ParsedFileSource` owners and
  function indices through replay. Only explicit mutable interpreter contracts,
  call environments and result matches become owned during solving.
- Encode extraction with `ParsedFile::encode_flatbuffer` into the containing
  module builder; standalone `WireEcco` is not production cache integration.

- Target statement-level bundler analysis for massive applications; throughput,
  memory usage, and parallel scalability are explicit design goals.
- Infer contracts from inputs and referenced symbols; already-defined dependency
  functions may be consulted. Persist inferred contracts during AST extraction;
  only closed, fully representable contracts replace lazy operation summaries.
  Keep linked captures and alias joins in owned summaries, never inferred guesses.
- Process dependencies before consumers and bound contract expansion; cycles and
  unsupported bodies produce unknown contracts whose execution stays deferred.
- Keep declarations and extracted graphs owned; never retain OXC arena data.
- Persist only `ParsedFile` precursors. Instantiate roots/functions in a fresh
  `Analysis`; external resolutions come from the caller's canonical graph.
- Register every file before solving; install all hoists before lazy root calls.
- Value IDs are context-local identities. Passthrough returns reuse them.
- Hazards only grow within a solve; record dependencies even on false facts.
- Evaluate against immutable rounds and merge writes before scheduling readers.
- Binding matching uses OXC semantics, never identifier spelling alone.
- Global reads default pure; callers declare effectful global symbol accessors.
- PURE annotations override invocation effects, not input evaluation or result hazards.
- Record function operations without executing them; rollback recording-only
  graph allocations and instantiate owned summaries at calls.
- Lexical binding slots are not value identities. Rebinding must not redirect
  existing aliases; each invocation owns its locals and closure captures.
- Returned closures retain exited-path capture alternatives across later paths.
- Intern namespace identities before recording; deferred reads share them.
- Keep unspecialized callable hazards separate from call-target mutation markers.
- Callback effect queries are latent; only execution applies external updates.
- Inferred effect queries subscribe to live captures and widen on rebinding.
- Unknown behavior must not become a proof of purity. Exceptions alone are not
  retention effects. This does not permit reordering or duplicating expressions.
