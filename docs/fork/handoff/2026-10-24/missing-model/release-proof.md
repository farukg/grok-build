# Gateway hotfix — activated and verified

## Result

The synthetic Responses envelope repair and Senku's semantic-idle/custom-tool-input work passed the complete Gateway suite, a fresh executable build, a separate canary, and the live service roundtrip. This repairs the independently proven invalid producer. Historical steering causality remains unproved.

- Source checkpoint, already committed by the user: `c618b3dfbc6b03e52ba584d6e0cf0cbfbdbf724c`.
- Full tests: `bld-1792704581745-6c57286a5cf14413a64b87420f5df417`, success, zero warnings, 63.231 seconds. Captured output: `gateway-tests-success.log`.
- Executable build: `bld-1792704839150-5d5479271d454085a45b987e9f44cb2c`, success, 149.755 seconds.
- Activated snapshot: `/home/fg/.local/state/sigma-services/sigma-gateway/releases/20261023-000958-sigma-gateway-e2128804.bin`.
- SHA-256: `e212880479b2a050973fa992138212086ad2960684c19a55801c19df4a5919eb`.
- Promotion output verified identical artifact, current snapshot and `/proc/2815618/exe` hashes. Original record: parent terminal `call_usUrnfHJzoAXIUuU53XwGyBJ.log`.

## Real requests

The separate canary used the normal service WorkingDirectory and port 6796. `canary-schema-roundtrip.log` records the successful real Sol text, forced tool call and returned-result requests. The owned canary was stopped after activation.

The first immediate post-restart probe reached the short pre-bind window and received connection refused. Subsequent `/health` returned HTTP 200. The complete live probe then exited zero after 74.18 seconds. `post-promote-schema-roundtrip.log` records three successful requests, valid required envelope fields, contiguous event sequences and exactly one completed terminal per request. The final text marker and actual tool output were checked.

The ten committed SDK regressions exercise real emitted relay bytes with the pinned async-openai client, including synthetic failure paths. The live requests cover the normal success path and do not inject a production transport failure.

## Source and remaining integration

Gateway files and Cargo.lock were clean against the user's checkpoint when inspected after activation. The shared main had eleven unpublished commits; this operation did not publish unrelated user checkpoints. The scoped pinned formatter subsequently passed after applying formatting-only changes. Its repeated full-suite check is currently blocked by five foreign Capture-Legacy ABI/import errors in request `bld-1792713431280-b73e6259aadb4eabb90d32c43685325e`; the responsible join is requested in `senku-observability-join.md`. Scoped publication remains open. The live binary remains the successfully tested pre-format artifact, independently identified by its hash.

The lockfile resolver canonically updated the host output-render version but returned a separate cargo-localbuild error resolving an inherited local package version. Subsequent locked Gateway tests and build succeeded. Tooling repair is tracked separately at `../cargo-update/result.md`.

The separate Grok steering, durable wakeup, delivery and display work remains open. Activating this Gateway fix is not a claim that those changes are staged.
