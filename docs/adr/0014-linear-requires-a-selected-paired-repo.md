# Linear requires a selected, authenticated and explicitly paired Repo

Linear is enabled only for the selected Repo after its connection has been
authenticated and that Repo has explicitly been paired with a Linear project.
An Owner's workflow block supplies defaults, not consent for all its Repos.
This supersedes milestone 2's Owner-wide activation and milestone 3's
cross-Repo Linear execution. ADR-0004's two write kinds still apply.

## Decision

- Until setup completes, normal screens offer only Connect Linear for the
  selected Repo. Missing credentials never prevent GitHub-only startup.
- Browser login uses the den-owned OAuth application with user authorization,
  PKCE, a checked random state, a bounded loopback callback and refresh tokens.
  The registered public client ID is included in den; a developer may override
  it with `DEN_LINEAR_OAUTH_CLIENT_ID`. It is not a secret.
- Users may instead enter an API key, select a populated `LINEAR_API_KEY` or
  configured key variable, or reuse schpet/linear-cli's documented
  `linear auth token` export. Discovery never pairs a Repo automatically.
- New secrets belong in the OS credential store. Environment and CLI secrets
  remain with their source; den never logs them or persists them in cache or
  configuration. No plaintext fallback is introduced.
- Pairings use canonical Repo identities and immutable workspace/project IDs.
  Credentials may be reused, but pairing is explicit for each Repo.
- Eligibility gates reads, cached data, UI, workflow policy and action dispatch.
  Switching Repo or changing a connection/pairing invalidates pending work;
  late replies cannot restore a disabled binding.
- Linear issue reads are restricted to the paired project. Start and Implement
  target only the selected Repo. A Land requiring another Repo is blocked,
  never partially executed. Excluded members are not treated as absent.
- Authentication rejection requires reconnection. Rate limits and network
  failures remain incomplete observations rather than successful validation.

## Consequences

Existing Owner-wide users must pair each Repo explicitly. Legacy unscoped
Linear cache entries do not confer eligibility; GitHub cache data is retained.
Disconnecting an account invalidates all pairings using it; unpairing affects
only the selected Repo. Project changes require confirmation and invalidate
cached and pending Linear work for the old pairing.

The den-owned public OAuth client is registered. A real browser consent and
callback round trip remains a separate validation requirement. Linux entered-key
and OAuth persistence requires the OS credential store through `secret-tool`;
environment keys remain usable without one. The supported CLI must implement
`auth token`; other tools named `linear` are not assumed to be compatible.

This decision introduces no additional Linear write kinds and does not change
den's GitHub read-only boundary. Multi-project pairing and cross-Repo execution
are outside this decision.
