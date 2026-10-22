# Ergänzungen und Bugs:




# Nebula – offene Ideen

Priorität: **A** = fehlt spürbar im Alltag, **B** = deutlicher Komfortgewinn, **C** = nice to have.

## Sicherheit und Datenerhalt

- **C** `.bak`-Kopie auf dem Server vor dem Überschreiben (eine lokale Kopie leistet schon der lokale Verlauf).

## Editor
- **C** Makros aufnehmen und abspielen.
- **C** Rechtschreibprüfung für Texte, Kommentare und Markdown (Windows-Rechtschreibprüfung nutzen).
- **C** Ligaturen in der Editorschrift (egui formt keine Ligaturen; bräuchte eigenes Text-Shaping).

## Suchen und Navigieren

- **C** Regex-Ersetzung mit Vorschau.
- **C** Gehe zu Definition ohne Language Server für einfache Fälle (CSS-Klassen, PHP-Funktionen im Projekt).

## Git

- **C** Blame für die aktuelle Zeile (Autor/Datum dezent am Zeilenende).

## Server und Veröffentlichen

- **C** Implizites FTPS (Port 990) – wird beim FileZilla-Import bisher als explizites FTPS übernommen.
- **C** Suche im Terminal-Verlauf (Strg+F im Terminal, Treffer im Scrollback hervorheben).
- **C** Datenbank light: MySQL über SSH-Tunnel, Tabellen ansehen, Abfragen ausführen.
- **C** Serverdateien zwischenspeichern, damit sie offline lesbar bleiben.

## Live-Vorschau und Web

- **C** Gerätebreiten der Vorschau: Querformat und eigene Breiten.

## KI-Unterstützung (wie in VS Code)

Alle abgestimmten KI-Punkte sind erledigt (siehe „Erledigt“): Agenten über ACP, eigener Agent, KI-Befehle, Commit-Nachricht, MCP-Server, Inline-Vorschläge, bearbeitbare Serverdateien, Codex-Limits.

## Oberfläche und Allgemein

- **C** Update-Hinweis (braucht eine Adresse, unter der neue Versionen veröffentlicht werden).
- **C** Barrierefreiheit: Bildschirmleser im Editor (AccessKit ist schon eingebunden), hoher Kontrast.

## Qualität und Technik

- **C** Große Textdateien im Hintergrund laden (bei ~70 MB blockiert das Öffnen die Oberfläche kurz); Hex-Editor: „Gehe zu Offset“, Suche, Bytes einfügen/löschen (Größe ändern).
- **C** Absturzbericht auch für harte Abstürze in nativem Code (Zugriffsverletzung in WebView/wgpu), z. B. per `SetUnhandledExceptionFilter` – bisher nur Rust-Panics.
- **C** Automatisierte UI-Tests für die wichtigsten Abläufe (Öffnen, Bearbeiten, Speichern lokal/Server).

## Nächste Schritte

Runde 1 und 2 sind abgeschlossen, dazu Zen-Modus, Schriftwahl, Farbwähler, Bildvorschau und Minimap.

Vorschlag für Runde 3 (kleinere B-Punkte zuerst):

1. ~~„Schnell öffnen“ (Strg+P) für Serverdateien und mit `:Zeile`~~ (erledigt).
2. ~~Absturzbericht~~ (erledigt).
3. ~~Server duplizieren, gruppieren, Import/Export~~ (erledigt).
4. ~~Vorschau in Gerätebreiten~~ (erledigt; kein eigener PHP-Server – dafür gibt es Herd, XAMPP und Co.).
5. ~~Eigene Snippets pro Sprache~~ (erledigt, als Panel „{ }“).
6. ~~Tabs anheften; Einstellungen exportieren/importieren~~ (erledigt).
7. ~~Große Dateien schreibgeschützt öffnen, Hex-Ansicht~~ (erledigt).
8. ~~Synchronisieren mit Vorschau; Installer/„Mit Nebula öffnen“~~ (erledigt).

Runde 3 ist damit komplett.

KI: alle abgestimmten Punkte erledigt (siehe oben).

## Erledigt

Externe Änderungen erkennen, Wiederherstellung nach Absturz, geteilte Ansicht, Blockauswahl, Formatieren beim Speichern und `.editorconfig`, zuletzt geöffnete Projekte, Git-Markierungen, Unterschiede im Konfliktdialog, eigene Tastenkürzel.

Danach: Knöpfe für geteilte Ansicht und rechtes Panel, rechtes Panel mit Vorschau und Gliederung, lokales Terminal (PowerShell, Git Bash, cmd), Wort unter dem Cursor hervorheben, passendes HTML-Tag hervorheben und mit umbenennen, Gehe zu Symbol (Strg+R), Navigationsverlauf (Alt+←/→), Lesezeichen, Textwerkzeuge, Suchen/Ersetzen in der Auswahl, Markierungen am rechten Rand, klickbare Pfadleiste (auch Netzlaufwerke werden jetzt korrekt geöffnet), Projektsuche mit Dateimustern und `.gitignore`, Projektsuche auf dem Server über SSH (abbrechbar), automatisch speichern, lokaler Verlauf, neues Fensterlayout mit Logo, Vergleich als Tab (Git-Markierungen funktionieren jetzt auch auf Netzlaufwerken), Git-Tab mit Commit, Pull/Push und Branches, Übertragungsliste (Neuverbindung nach Verbindungsabbruch repariert), gespeicherte Serverbefehle, Zen-Modus, Schriftart und Zeilenhöhe, Farbwähler für CSS-Farben, Bildvorschau, Minimap.

Bugrunde vom 19.10.: Rechtsklick auf den Projektordner, Startseite (Karte wächst mit, Listen leeren), ICO-Dateien, Bilder in der Vorschau, Zurück/Vor und schmale Scrollleisten in der Vorschau, Dateifilter ohne Einfrieren und zentriert, Schriftart-Dropdown, gleich große Einstellungsseiten.

KI-Agenten (19.10.): Claude Code, Codex (ChatGPT) und Gemini CLI über ACP im rechten Panel (Reiter „KI“): Verlauf mit Markdown, Plänen, Werkzeugaufrufen und Diffs, Rückfragen, Stop; Modell, Aufwand und Modus wählbar (pro Agent gemerkt); Dateien anhängen (Büroklammer, Rechtsklick in der Seitenleiste, auch vom Server); 5-Stunden- und Wochenlimit von Claude; Einstellungsseite „KI-Agenten“ mit Anmeldung über die Anbieter, API-Schlüsseln und Node.js-Verwaltung (Node wird bei Bedarf nach `%APPDATA%\nebula\runtime` geladen, Agenten liegen in `%APPDATA%\nebula\agents`); Änderungen des Agenten werden bei „Beim Speichern hochladen“ auf den Server übertragen. Außerdem: Reiter im rechten Panel jetzt Gliederung, KI, Vorschau.

Bugrunde vom 20.10.: Theme je für helles und dunkles Design mit Umschalter Hell/Dunkel/Automatisch (Einstellungen, Menü „Ansicht“, Knopf in der Titelleiste), Menüeintrag „KI-Agenten…“, Terminal-Knopf in der Titelleiste, geteilte Ansicht als Knopf in der (jetzt so hohen wie die Panel-Reiter) Pfadleiste, ID-Kollisionsmeldungen in den Einstellungen von Erweiterungen (z. B. Rust), Knopf für die Minimap neben dem für die geteilte Ansicht.

Serververwaltung (22.10.): Server in einklappbaren Gruppen, duplizieren, Import aus FileZilla (`sitemanager.xml`, Ordner als Gruppen, Passwörter auf Wunsch) und aus Nebula-Exporten, Export als JSON ohne Passwörter; Dubletten werden erkannt.

Schnell öffnen (22.10.): auch Dateien aus geladenen Serverordnern, `datei:Zeile[:Spalte]` und `:Zeile` für den aktuellen Tab; Treffer im Dateinamen werden besser gewichtet.

Absturzbericht (22.10.): Panics landen mit Meldung, Stelle und Backtrace in `crashes\` (Release-Build mit Zeilentabellen in `nebula.pdb`), beim nächsten Start Dialog mit Öffnen/Kopieren/Explorer; Testauslöser `NEBULA_CRASH_TEST=1`.

Gerätebreiten (22.10.): Vorschau wahlweise in Panelbreite, Handy (390 px), Tablet (768 px) oder Desktop (1440 px), bei Bedarf verkleinert (WebView-Zoom).

Snippets (22.10.): Tab-Sprünge durch Platzhalter mit Mehrfachbearbeitung gleicher Nummern, eingebaute Snippets mit Platzhaltern, eigener Reiter „{ }“ im rechten Panel zum Einfügen, Anlegen (auch aus der Auswahl) und Bearbeiten, pro Sprache oder für alle Sprachen. Export/Import als JSON, Import von VS-Code-Snippets.

Einstellungen exportieren/importieren (22.10.): alle Bereiche einzeln wählbar in einer JSON-Datei, ohne Passwörter und API-Schlüssel; Import ergänzt Server/Snippets/Agenten und installiert fehlende Erweiterungen aus der Bibliothek nach.

Tabs anheften (22.10.): angeheftete Tabs stehen links mit Nadel statt Schließen-Knopf, bleiben bei „Andere/Rechts/Alle schließen“ und Mittelklick stehen und werden in Sitzung und Projekt-Tabs gemerkt.

Große Dateien und Hex-Editor (22.10.): Binärdateien öffnen im Hex-Editor (Überschreiben, Rückgängig, Speichern nur der geänderten Bytes) (große lokale Dateien werden stückweise gelesen, „Als Text öffnen“ bei Fehlerkennung), Textdateien über 64 MB schreibgeschützt bis 512 MB, darüber als Hex.

Synchronisieren (22.10.): Projekt und Serverordner in beide Richtungen abgleichen – Vorschau mit Hochladen/Herunterladen/Löschen/Konflikten, Stand der letzten Synchronisierung wird gemerkt, Löschungen nicht vorausgewählt (lokal in den Papierkorb), Ausschlussliste pro Projekt.

Kopieren zwischen Servern (22.10.): Ziehen zwischen verschiedenen Orten kopiert (Umschalt verschiebt), Hinweis am Mauszeiger, Kontextmenü „Auf Server kopieren/verschieben ›“; „Veröffentlichen“ und „Beim Speichern hochladen“ beachten jetzt die Ausschlussliste (z. B. `.env`).

Serverliste (22.10.): Server per Drag \& Drop in Gruppen sortieren, Umschalter zwischen Gruppen und flacher Liste.

Snippet-Gruppen (22.10.): eigene Snippets in einklappbaren Gruppen (Feld „Gruppe“, Drag \& Drop), Gruppen in Export\/Import und Einstellungs-Export; VS-Code-Sammlungen (`*.code-snippets`) werden zu einer Gruppe.

Windows-Integration (22.10.): Installieren pro Benutzer ohne Fremdwerkzeug (Programmordner, Startmenü, Eintrag unter „Apps“), „Mit Nebula öffnen“ im Explorer, „Öffnen mit“ für 44 Text-/Code-Dateitypen, zweiter Start öffnet Dateien im laufenden Fenster.

Neue Datei mit Sprache (22.10.): Rechtsklick oder Umschalt+Klick auf „+“ wählt Sprache und Endung; „Speichern unter“ schlägt die Endung der Dokumentsprache vor.

Lokalisierung (22.10.): Oberfläche auf Deutsch, Englisch, Spanisch, Französisch und Italienisch; automatisch nach der Windows-Anzeigesprache (sonst Englisch) oder fest unter Einstellungen › Editor. Sprachdateien (`src/i18n/*.json`, deutscher Text als Schlüssel) lassen sich exportieren, bearbeiten und importieren; eigene liegen in `languages\` im Einstellungsordner. Werkzeug `tools/i18n-wrap` (keys/merge) für neue Texte. Sprachwechsel wirkt sofort, „Automatisch“ nennt die tatsächlich gewählte Sprache; feste Kürzel in Menüs (Strg+…) werden mitübersetzt.

Kopieren auf den Server und Herunterladen ins Projekt (22.10.): „Auf Server kopieren/verschieben › Server ›“ mit „In ausgewählten Ordner“ (zuletzt auf dem Server angeklickter Ordner), „In Startordner“ und „In Ordner…“ (Ordnerauswahl auf dem Server); bei Serverdateien „Herunterladen in Projektordner ›“ mit „In ausgewählten Ordner“, „In Projektordner“ und „In Ordner…“. Vorhandene Namen fragen jetzt auch bei noch nicht aufgelisteten Serverordnern nach (Ersetzen/Beide behalten/Überspringen, auf Wunsch für alle), statt stillschweigend zu überschreiben.

Fehlersuche Datenverlust (22.10.): Neue Datei auf dem Server mit vorhandenem Namen leerte die vorhandene Datei; Umbenennen auf einen vorhandenen Namen überschrieb lokal (Windows) und per FTP stillschweigend; Tabs in umbenannten Ordnern zeigten auf den alten Pfad (und blieben nach fehlgeschlagenem Umbenennen auf dem neuen); Synchronisieren mit unvollständig eingelesener Seite (nicht lesbarer Ordner, Limit erreicht) hätte vorhandene Dateien als „neu“ überschrieben bzw. Löschen vorgeschlagen – jetzt Abbruch mit Meldung; Löschen in der Seitenleiste geht lokal in den Papierkorb.

Sicheres Speichern und Inhaltsvergleich (22.10.): Lokales Speichern (auch „In Dateien ersetzen“ und KI-Agent) schreibt erst in eine Hilfsdatei und ersetzt dann in einem Schritt (`ReplaceFileW`); die erste Synchronisierung vergleicht gleich große Dateien inhaltlich, statt sie für identisch zu halten.

Lesezeichen-Panel (22.10.): Doppelklick auf eine Zeilennummer setzt/entfernt ein Lesezeichen; Reiter „Lesezeichen“ im rechten Panel mit allen Lesezeichen der offenen Dateien (Zeilentext, Klick springt, × entfernt, „Alle entfernen“); Lesezeichen werden in Sitzung und Projekt-Tabs gespeichert.

Eigener KI-Agent (22.10.): „In Nebula eingebaut“, ohne Node.js – Anthropic, OpenAI und OpenAI-kompatible Dienste (eigene Server-Adresse) sowie Ollama; spricht intern ACP, daher dieselbe Oberfläche wie die anderen Agenten (Modellwahl aus der Liste des Anbieters, Werkzeugaufrufe, Diffs, Rückfrage, Stopp). Werkzeuge: Datei lesen (offene Tabs), Dateien auflisten, Projekt durchsuchen, Datei ändern/schreiben (Diff mit Übernehmen/Immer/Ablehnen), Befehl in cmd.exe (mit Rückfrage, 2 Minuten Zeitlimit).

KI-Befehle (22.10.): Rechtsklick im Editor › „KI“ (und Befehlspalette): Erklären, Kommentieren, Umschreiben… (mit Anweisung, Vorschläge wie „lesbarer“), Fehler beheben (mit den gemeldeten Problemen) – für die Auswahl oder die aktuelle Zeile; geht an den Agenten im Panel, Änderungen als Diff zum Übernehmen. Ändern nur bei lokalen Dateien. Nebenbei: Markdown im Agent-Panel bricht lange Absätze jetzt um.

Commit-Nachricht per KI (22.10.): ✦ im Nachrichtenfeld des Git-Tabs – der Agent des Panels schreibt die Nachricht aus dem Diff (vorgemerkte Änderungen, sonst alle samt neuer Dateien) in einer eigenen, unsichtbaren Sitzung; Sprache und Stil wie die letzten Commits; darf Dateien lesen, nichts ändern; nochmal klicken bricht ab.

Nebula als MCP-Server (22.10.): lokaler MCP-Server (HTTP, nur 127.0.0.1, Schlüssel in der Adresse) mit get_open_files, get_active_editor (Cursor, Auswahl), get_diagnostics, get_file_text (ungespeicherter Stand) und open_file; Agenten im Panel bekommen ihn automatisch, für Claude Code im Terminal gibt es den Befehl zum Kopieren (Einstellungen › KI-Agenten), dort auch Aus und „Neuer Schlüssel“.

Inline-Vorschläge (22.10.): nach einer Tipp-Pause am Zeilenende fragt Nebula einen eingebauten Agenten (eigenes, sonst automatisch kleines Modell wie Haiku/mini); Vorschlag grau am Cursor, weitere Zeilen auf Hintergrund darunter; Tab übernimmt, Esc verwirft, Weitertippen/Cursor bewegen lässt ihn verschwinden. Standardmäßig aus (Einstellungen › KI-Agenten), Pause einstellbar.

Serverdateien für Agenten bearbeitbar (22.10.): Server-Dateien bekommen für den Agenten einen Pfad `\\nebula\<Server-ID>\pfad`; Lesen liefert den offenen Tab oder holt die Datei vom Server, Schreiben geht in den offenen Tab (Speichern = Hochladen) oder direkt auf den Server; angehängte Serverdateien nennen dem Agenten diesen Pfad; KI-Befehle zum Ändern gehen jetzt auch für Serverdateien; Klick auf solche Dateien im Panel öffnet die Serverdatei.

Codex-Limits (22.10.): verbleibende Limits des ChatGPT-Abos (5 Stunden, Woche) unter der Eingabe wie bei Claude – über `/status` des Codex-Adapters in einer unsichtbaren Sitzung, ohne Modellaufruf; Reset-Zeit in Ortszeit, Datum unabhängig von der Systemsprache.

Mehrere Fenster (22.10.): „Neues Fenster“ (Strg+Umschalt+N), „Ordner in neuem Fenster öffnen…“ (Menü Datei, Projekt-Umschalter, Befehlspalette), Strg+Klick auf zuletzt geöffnete Projekte; jedes Fenster ist ein eigener Prozess mit eigenem Projekt. settings.json und sites.json werden beim Speichern dreiseitig zusammengeführt (jedes Fenster schreibt nur, was es selbst geändert hat) und von den anderen Fenstern binnen einer Sekunde neu eingelesen; Sitzung und letzter Ordner gehören dem Hauptfenster, weitere Fenster merken sich ihre Tabs pro Projekt. Fenstertitel nennt das Projekt.

Performance (22.10.): Highlighting großer Dateien (ab 256 KB) inkrementell – der Syntaxbaum wird weitergeführt, nur geänderte Zeilen neu gefärbt (3 MB: 1,2 s → 30 ms pro Tastendruck), Farben jetzt bis 16 MB statt 4 MB; minifizierte Dateien mit einer 5-MB-Zeile hängen nicht mehr (Wort- und Suchmarkierungen nur im sichtbaren Ausschnitt, schnellere Spaltenberechnung); Wortvervollständigung, Gliederung (im Hintergrund), Zeilenumbruch, Git-Markierungen und Breitenmessung belasten das Tippen in großen Dateien nicht mehr. Getestet mit 3/5/20/50 MB und 120 Tabs. Zeitmessung pro Frame mit `NEBULA_PERF=1`. Dabei behoben: Code-Blöcke in Markdown wurden falsch gefärbt (Satzzeichen fehlten im eingebetteten Code, CSS-Eigenschaften erschienen als Selektoren).
