---
name: nebula-i18n
description: Wie Nebulas Übersetzungen funktionieren und wie neue Texte/Sprachen nachgezogen werden
metadata:
  node_type: memory
  type: project
  originSessionId: 5de3a7ed-4e7b-4c68-bf11-2d48ee8ae9ef
  modified: 2026-10-22T11:58:16.470Z
---

Nebula-UI ist lokalisiert (Stand 2026-10-22): Deutsch ist Quelle UND Schlüssel. `t!("Text")` / `t!("{n} x", n = v)`, `tk!` für const-Tabellen (Anzeige mit `crate::i18n::tr`), feste Kürzel-Texte über `crate::i18n::keys("Strg+…")`. In src/i18n.rs selbst `tr()` statt `t!` (Makro ist dort erst weiter unten definiert). Dateien src/i18n/{en,es,fr,it}.json; Nutzerdateien in config_dir()/languages überschreiben. Fallback: Sprache → Englisch → Deutsch. Anrede: du/tu/tú.

Neue Texte nachziehen: `tools/i18n-wrap` (eigenes Cargo-Projekt) – `keys src` listet Schlüssel, `merge <keys.json> <tsv> <code> <name> <out>` baut JSON aus `N<TAB>Text` (Escapes `\n`, `\\`, `\"`). merge ERSETZT die Datei komplett, also immer alle TSV-Teile zusammen übergeben. Das wrap-Werkzeug übersprang Texte mit Bindestrich-Wörtern und `Strg+`-Literale – bei neuem Code direkt `t!` schreiben. Der Test `builtin_files_parse` prüft gleiche Platzhalter.

Nicht übersetzen: Absturzbericht (wird geparst), gespeicherte Shell-Namen („Eingabeaufforderung“), Theme-IDs „Nebula Hell/Dunkel“.

**Why:** Damit spätere Änderungen an UI-Texten nicht unübersetzt bleiben.
**How to apply:** Bei jedem neuen UI-Text `t!` nutzen und die vier JSONs ergänzen; siehe [[nebula-workflow]].
