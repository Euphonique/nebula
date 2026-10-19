---
name: nebula-progress
description: Stand der Arbeit am Editor Nebula (A:\Development\Nova\nebula) und wo als Nächstes weitergemacht wird
metadata:
  node_type: memory
  type: project
  originSessionId: 10a7e079-fb2f-49fa-8901-631ac611494f
  modified: 2026-10-19T21:14:36.096Z
---

Nebula = nativer Windows-Code-Editor in Rust (eframe/egui 0.37), Nova-ähnlich, mit FTP/SFTP. Aufgaben stehen priorisiert (A/B/C) in `todo.md`; „Nächste Schritte“ enthält die abgestimmte Reihenfolge. Erledigtes wird aus der Liste entfernt und unter „Erledigt“ ergänzt, README wird mitgepflegt. Der Nutzer sammelt gefundene Bugs gelegentlich als Liste ganz oben in todo.md.

Stand 2026-10-19 abends: alle 94 Tests grün (6 ignorierte Netz-/Server-Tests), Arbeitsverzeichnis committet, Release-Build aktuell. Der Nutzer startet `target/release/nebula.exe` – nach Änderungen `cargo build --release` im Hintergrund laufen lassen (ca. 4–5 Min.).

Fertig (Auswahl, mit Orten im Code):
- Runde 1 und 2, u. a. Pfadleiste (src/app/jumpbar.rs), Projektsuche lokal + SSH (src/ui/search.rs, src/remote/search.rs), Autosave + lokaler Verlauf (src/history.rs), Vergleich als Tab (src/app/difftab.rs), Git-Tab (src/ui/git_panel.rs), Übertragungsliste, Serverbefehle, Zen-Modus, Schriftwahl (src/fonts.rs), Farbwähler, Bildvorschau (src/image_view.rs, eigener ICO-Leser), Minimap.
- Bugrunde 19.10. (Startseite, Filter im Hintergrund src/ui/file_filter.rs, Vorschau mit Bildern/Zurück/Vor, Einstellungs-Dropdowns).
- KI-Agenten über ACP, Details siehe [[nebula-ki-scope]].

Nächste Arbeit: Nach Wunsch des Nutzers entweder KI weiter (eigener Agent für API-Schlüssel/Ollama, dann KI-Befehle, Commit-Nachricht, MCP, Inline) oder „Runde 3“ aus todo.md (Strg+P für Serverdateien, Absturzbericht …). Vorher kurz fragen, womit er weitermachen will.
Das Claude-Wochenlimit des Nutzers war am 19.10. fast aufgebraucht (3 % frei, Reset 22.10. 03:00) – er achtet darauf und bittet am Ende um Memory-Update.

**Why:** Damit die nächste Sitzung ohne Nachfragen zum Stand weitermachen kann.
**How to apply:** Pro Aufgabe ein Commit (siehe [[nebula-workflow]]); todo.md und README mitpflegen.
