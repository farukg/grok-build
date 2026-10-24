# Release evidence — 2026-10-22

## Grok
Commit 0c738e544879ec5ca12df4956a4986c70ad69287 published to farukg/grok-build.
linux-build run39694698998 and linux-test run39694700000 completed success for same SHA (captured linux-build.yml.json and linux-test.yml.json).
Exact-run stage completed, archive SHA verification OK; output grok-stage.log.
Selected release /home/fg/.local/state/sigma-binaries/grok/releases/20261022-022534-xai-grok-pager-6fda647f.bin.
Version grok1.0.45 (0c738e544879) alpha.
SHA256 6fda647f5faa98397eb15880bcae774db08c1515f9e17589f9253e001b399945.
Two fresh actual headless CLI runs with only read_file enabled passed exit 0 and exact TOOL_READ_CONFIRMED stdout. Evidence grok-smoke/run-1.stdout, run-2.stdout, stderr and evidence.sha256. Existing user runtime/session config untouched.
User notified staged/restart-ready. Separate newly requested child-compaction repair remains in progress and is not included in this release.

## Gateway
Independent required-sequence commit475cfb9750a70b27e4f62c3ec099b5761850ca28.
Full sigma-gateway tests passed request bld-1792628318945-4fc0335cc2fd4609b1c493912a19a9d4; log gateway-sequence-success.log.
Release binary built successfully request bld-1792628898048-988db0e1f2de4d4787e1efa724ff5797; artifact /home/fg/.cache/cargo-target/quick/release/sigma-gateway.
Five unused local patches appended by Cargo to Cargo.lock; normal-hook retry succeeded commit8ef21605. Main fast-forward integrated and published, preserving foreign work.
Two strict fresh canary startups passed actual text/forced function argument and monotonic required response sequence/terminal checks. Tool-output followup failed HTTP400; cause under exact-response investigation, not claimed passed.
Live promotion succeeded with artifact hash check: selected release /home/fg/.local/state/sigma-services/sigma-gateway/releases/20261022-033907-sigma-gateway-39d36abf.bin. sig deploy verified process. Evidence gateway-promotion.log, gateway-active-release.txt, gateway-active-process.txt and gateway-active.sha256. Actual live6790 strict final check passed: canary/live6790-final.log contains health200, exact output-text marker, forced probe function arguments, matched function_call/output replay and consumed returned-result marker; all streams monotonic response.* sequence, exactly one completed terminal. Earlier HTTP400 was malformed probe string input; corrected typed message pass. Native synthetic failure proof is committed production-relay unit tests over every byte split; actual binary local upstream injection unsupported, precise limitation failure-probe/result.md.
