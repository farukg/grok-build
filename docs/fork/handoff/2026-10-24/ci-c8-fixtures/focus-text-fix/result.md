# Fokusgleicher Composervergleich

Nur `send_subagent_message_row.rs` geändert: Snapshot vor Tab gegen nach Composer-Klick; bisheriger unfokussierter Snapshot gegen nach go-Klick. Keine Placeholder-Filterung oder zusätzliche Toleranz. Deselektion, neuer go-Rahmen, Draftvergleich, gemeinsame 60s-Deadline und Child-/Caret-/Enter-/ID-/Session-Infooracles bleiben erhalten. Alle anderen Sourcepfade wurden nicht editiert.

Basis: `before-send_subagent_message_row.rs`, `before-source-hashes.json`, `before-source-manifest.json`, `before-fixtures.patch`. Exakter einzelner Hunk: `focus-text-fix.patch`. Finale Gesamtmetadaten: `../source-manifest.json`.

- source_sha256: `94139f49e049cc6b618e465e5d95a9409ebf91dde12142f95d7833a3c1d5f15e`.
- fixtures_patch_sha256: `536e80dc21055a42c51d14b15af34ff0b8105976042fb652342d33c1c2487589`.
- single_hunk_patch_sha256: `34b538a896eef5ae78010bc5dde3740b8a59e23045dd2b0a0766c90f2df634e8`.

Keine Builds, Tests, Checks, cargo, Formatierung, Commits oder Publikation ausgeführt. Nur Scratch-Patch-/Hashausgabe. Erneute unabhängige Reviewfreigabe sowie Kompilierung/Runtime bleiben offen.
