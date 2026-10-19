---
name: nebula-progress
description: Stand der Arbeit am Editor Nebula (A:\Development\Nova\nebula) und wo als Nächstes weitergemacht wird
metadata:
  node_type: memory
  type: project
  originSessionId: 54481c14-f69f-4772-8ba1-6fa54422a0f2
  modified: 2026-10-18T21:27:45.981Z
---

Nebula = nativer Windows-Code-Editor in Rust (eframe/egui 0.37), Nova-ähnlich, mit FTP/SFTP. Aufgaben stehen priorisiert (A/B/C) in `todo.md`; der Abschnitt „Nächste Schritte“ enthält die abgestimmte Reihenfolge. Erledigte Punkte werden aus der Liste entfernt und unter „Erledigt“ ergänzt, README wird mitgepflegt.

Stand 2026-10-19: Bugrunde des Nutzers (11 Punkte oben in todo.md) erledigt und aus todo.md entfernt, alle 85 Tests grün. Der Nutzer sammelt neue Bugs gelegentlich als Liste ganz oben in todo.md. Der Release-Build `target/release/nebula.exe` ist aktuell – der Nutzer startet diese exe, also nach Änderungen `cargo build --release` laufen lassen (dauert ca. 4 Min., im Hintergrund starten).

Fertig (Auswahl, mit Orten im Code):
- Runde 1 und 2 komplett, u. a. Pfadleiste (src/app/jumpbar.rs), Projektsuche lokal + per SSH (src/ui/search.rs, src/remote/search.rs), Autosave + lokaler Verlauf (src/history.rs, src/app/history.rs), Vergleich als Tab (src/app/difftab.rs), Git-Tab (src/ui/git_panel.rs), Übertragungsliste (src/remote/transfers.rs, src/app/transfers_ui.rs), Serverbefehle (Site.commands, „Befehle ▾“ im Terminal).
- Danach: neues Fensterlayout mit Logo und Agency-Schriftzug als Vektor (src/ui/wordmark.rs), Zen-Modus, Schriftwahl (src/fonts.rs), CSS-Farbwähler (src/editor/colors.rs), Bildvorschau (src/image_view.rs; Bild-Tabs werden nie gespeichert), Minimap (src/editor/view.rs).

Nächste Arbeit: „Runde 3“ laut todo.md, beginnend mit Strg+P für Serverdateien und `:Zeile`, dann Absturzbericht usw.
KI im rechten Panel (ACP-Agenten wie Zed, eigener Chat) hat der Nutzer bewusst auf später verschoben – nicht von selbst anfangen, erst den Umfang abstimmen.
Das Wochenlimit war bei Sitzungsende knapp (ca. 20 % übrig); der Nutzer achtet darauf und bittet am Ende um Memory-Update.

**Why:** Damit die nächste Sitzung ohne Nachfragen weitermachen kann.
**How to apply:** Mit dem ersten Punkt von „Runde 3“ in todo.md starten, pro Aufgabe ein Commit (siehe [[nebula-workflow]]).
