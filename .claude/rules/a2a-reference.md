# A2A Protocol Reference

When working on this project, always reference the A2A protocol specification files:

- **Protobuf definition**: `A2A/specification/a2a.proto` — normative source for all protocol data objects and request/response messages
- **Full specification**: `A2A/docs/specification.md` — detailed technical specification including operations, data model, protocol bindings, and security
- **Agent discovery**: `A2A/docs/topics/agent-discovery.md` — discovery strategies (Well-Known URI, Registries, Direct Configuration)

These files are the authoritative reference. Always verify design decisions and implementations against them.

## Related Codebases

- `A2A/` — Google A2A protocol specification repository (reference only, do not modify)
- `solace-agent-mesh/` — Solace Agent Mesh framework (reference for related work comparison, do not modify)
