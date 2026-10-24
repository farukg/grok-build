# Grok-Auslieferungsabgleich

## Direkt geprüft

- GitHub-Lauf: https://github.com/farukg/grok-build/actions/runs/40000493080
- Workflow: linux-build; Branch: main; Status: completed; Ergebnis: success.
- Vollständiger Commit: `c8e075ea29ab74356cb477f05dcd70cdf7eb0e4c`.
- `grok --version`: `grok 1.0.45 (c8e075ea29ab) [alpha]`.
- Downloadartefakt: `/home/fg/.cache/cargo-target/quick/release/xai-grok-pager`.
- SHA-256: `877d52ef5fe0f5108d466fa09112fb4fd3278b53b0aba1ebb5c9e5a3e2effdb6`.

## Übermittelte Stage-Evidence

Tintin meldet erfolgreichen gebundenen Stage über `stage-grok-release.sh` mit Repo-/Workflow-/Branch-/Head-/Conclusionprüfung und Artefaktchecksumme sowie anschließender Promotion durch `sig deploy stage grok`. Sein Stage-Log wurde in diesem Abgleich noch nicht gelesen. Die lokal direkt erhobenen Versions- und Artefakthashwerte stimmen mit seiner Meldung überein.

## Offene Abnahmen

Dieser Buildbeleg ersetzt keine vollständige linux-test-Abnahme für denselben Commit. Die direkte Abfrage `gh run list --repo farukg/grok-build --workflow linux-test.yml --commit c8e075ea29ab74356cb477f05dcd70cdf7eb0e4c --limit 5 --json databaseId,status,conclusion,headSha,url` endete mit Exit 0 und leerer Ergebnisliste `[]`; ein vorhandener linux-test-Lauf für diesen SHA ist damit derzeit nicht belegt. Der erfolgreiche Stage belegt ebenfalls keine Übernahme des neuen Codes durch bereits laufende Prozesse. Spätere Registrierungs-/Zustelländerungen benötigen einen eigenen exakten Quell- und CI-Abgleich. Reale Feature-/PTY-Prüfungen und Registrierung-Producer-/Consumer-Rollout bleiben offen.

## Nachfolgend eingereichte Testabnahme

Main wurde vor dem Dispatch erneut auf den vollständigen SHA `c8e075ea29ab74356cb477f05dcd70cdf7eb0e4c` geprüft. Die letzten fünf linux-test-Läufe waren abgeschlossen; es gab keinen aktiven Lauf in der geteilten Concurrencygruppe. `gh workflow run linux-test.yml --repo farukg/grok-build --ref main` endete mit Exit 0 und lieferte den neuen Lauf https://github.com/farukg/grok-build/actions/runs/40003673652.

Der bestehende ereignisgebundene Monitor `compaction-pty/watch-ci.py` verfolgt exakt diesen Lauf und erwartet denselben vollständigen SHA. Er meldet jede fehlgeschlagene Step-/Job-/Runconclusion sowie einen SHA-Konflikt sofort mit FAILED. Laufabschluss ist ausstehend. Der Defaultlauf prüft alle Pager-/Chat-State-/Sampling-Types-Libtests sowie benannte Sampler-/Shell-/Tools-/Compaction-/PTY-Pfade; der allgemeine Shell-Libstep bleibt im bestehenden Workflow auf `remove` gefiltert. Daraus wird keine vollständige Shellsuite behauptet.

## Befehlsprovenienz

- `gh run view 40000493080 --repo farukg/grok-build --json headSha,headBranch,status,conclusion,workflowName,url`, Exit 0.
- `grok --version`, Exit 0.
- `sha256sum /home/fg/.cache/cargo-target/quick/release/xai-grok-pager`, Exit 0.

Keine Builds, Tests, Stageoperation oder zusätzliche Publikation durch diesen Abgleich.
