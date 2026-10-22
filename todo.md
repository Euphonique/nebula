# Nebula – offene Ideen

Priorität: **A** = fehlt spürbar im Alltag, **B** = deutlicher Komfortgewinn, **C** = nice to have.

## Sicherheit und Datenerhalt

- **C** `.bak`-Kopie auf dem Server vor dem Überschreiben (eine lokale Kopie leistet schon der lokale Verlauf).

## Editor
- **B** Eigene Snippets pro Sprache in den Einstellungen anlegen/bearbeiten (mit Platzhaltern `$1`, `${2:name}`).
- **C** Makros aufnehmen und abspielen.
- **C** Rechtschreibprüfung für Texte, Kommentare und Markdown (Windows-Rechtschreibprüfung nutzen).
- **C** Ligaturen in der Editorschrift (egui formt keine Ligaturen; bräuchte eigenes Text-Shaping).
- **C** Lesezeichen im Panel rechts einbauen. Doppel Klick auf Zeilennummer setzt/entfernt Lesezeichen (mit Markierung bei Zeilennummer) und trägt dieses in die Liste der Lesezeichen im rechten Panel ein. Mit Klick auf ein Lesezeichen im rechten Panel springt es im Editor an besagte Stelle.

## Suchen und Navigieren

- **C** Regex-Ersetzung mit Vorschau.
- **C** Gehe zu Definition ohne Language Server für einfache Fälle (CSS-Klassen, PHP-Funktionen im Projekt).

## Git

- **C** Blame für die aktuelle Zeile (Autor/Datum dezent am Zeilenende).

## Server und Veröffentlichen

- **B** Synchronisieren in beide Richtungen mit Vorschau (neu/geändert/gelöscht) und Ausschlussliste (`.git`, `node_modules`, `.env`).
- **C** Implizites FTPS (Port 990) – wird beim FileZilla-Import bisher als explizites FTPS übernommen.
- **C** Suche im Terminal-Verlauf (Strg+F im Terminal, Treffer im Scrollback hervorheben).
- **C** Datenbank light: MySQL über SSH-Tunnel, Tabellen ansehen, Abfragen ausführen.
- **C** Serverdateien zwischenspeichern, damit sie offline lesbar bleiben.

## Live-Vorschau und Web

- **B** Vorschau in verschiedenen Gerätebreiten (Handy/Tablet/Desktop) umschaltbar.

## KI-Unterstützung (wie in VS Code)

Agenten über ACP sind fertig (siehe „Erledigt“). Offen, in dieser Reihenfolge abgestimmt:

- **B** Eigener KI-Agent ohne Node für API-Schlüssel und lokale Modelle: Anthropic, OpenAI (auch kompatible Basis-URL), Ollama; Werkzeuge Datei lesen, Ändern als Diff, Suchen, Befehl mit Rückfrage. (Abo-Logins gehen nur über die Agenten der Anbieter – Anthropic sperrt fremde OAuth-Nutzung, nur Haiku erreichbar.)
- **B** KI-Befehle auf der Auswahl: Erklären, Kommentieren, Umschreiben/Refaktorisieren, Fehler beheben – Ergebnis als Diff zum Übernehmen.
- **B** Commit-Nachricht aus dem Diff (Knopf im Git-Tab).
- **C** Nebula als MCP-Server: externe Agenten sehen offene Dateien, Auswahl und Fehler und können Dateien im Editor öffnen.
- **C** Inline-Vorschläge beim Tippen (graue Vervollständigung, Tab übernimmt) – unbedingt ein-/ausschaltbar (Kosten).
- **C** Angehängte Serverdateien direkt bearbeitbar machen (virtueller Pfad, Lesen/Schreiben über die Serververbindung); bisher nur lesend bzw. über ein lokales Projekt mit „Beim Speichern hochladen“.
- **C** Limits-Anzeige auch für Codex (ChatGPT-Abo), sobald der Adapter sie meldet.

## Oberfläche und Allgemein

- **B** Tabs anheften; mehrere Fenster/Projekte gleichzeitig.
- **B** Große Dateien (> 64 MB) schreibgeschützt öffnen statt ablehnen; Hex-Ansicht für Binärdateien.
- **B** Einstellungen exportieren/importieren (Themes, Tastenkürzel; die Serverliste lässt sich schon exportieren).
- **B** Update-Hinweis, Installer und Eintrag „Mit Nebula öffnen“ im Explorer-Kontextmenü, Dateizuordnungen.
- **C** Oberfläche auch auf Englisch (Texte in Übersetzungsdateien auslagern).
- **C** Barrierefreiheit: Bildschirmleser im Editor (AccessKit ist schon eingebunden), hoher Kontrast.

## Qualität und Technik

- **B** Performance mit sehr großen Dateien (10–50 MB, minifiziertes JS in einer Zeile) und vielen Tabs prüfen; Highlighting inkrementell statt komplett neu.
- **B** Absturzbericht: Panics in eine Logdatei schreiben und beim nächsten Start anbieten.
- **C** Automatisierte UI-Tests für die wichtigsten Abläufe (Öffnen, Bearbeiten, Speichern lokal/Server).

## Nächste Schritte

Runde 1 und 2 sind abgeschlossen, dazu Zen-Modus, Schriftwahl, Farbwähler, Bildvorschau und Minimap.

Vorschlag für Runde 3 (kleinere B-Punkte zuerst):

1. ~~„Schnell öffnen“ (Strg+P) für Serverdateien und mit `:Zeile`~~ (erledigt).
2. Absturzbericht (Panics in Logdatei, beim nächsten Start anbieten).
3. ~~Server duplizieren, gruppieren, Import/Export~~ (erledigt).
4. Vorschau in Gerätebreiten (kein eigener PHP-Server – dafür gibt es Herd, XAMPP und Co.).
5. Eigene Snippets pro Sprache.
6. Tabs anheften; Einstellungen exportieren/importieren.
7. Große Dateien schreibgeschützt öffnen, Hex-Ansicht.
8. Synchronisieren mit Vorschau; Installer/„Mit Nebula öffnen“.

KI: als Nächstes der eigene Agent für API-Schlüssel und Ollama, danach KI-Befehle, Commit-Nachricht, MCP-Server, Inline-Vorschläge (siehe oben).

## Erledigt

Externe Änderungen erkennen, Wiederherstellung nach Absturz, geteilte Ansicht, Blockauswahl, Formatieren beim Speichern und `.editorconfig`, zuletzt geöffnete Projekte, Git-Markierungen, Unterschiede im Konfliktdialog, eigene Tastenkürzel.

Danach: Knöpfe für geteilte Ansicht und rechtes Panel, rechtes Panel mit Vorschau und Gliederung, lokales Terminal (PowerShell, Git Bash, cmd), Wort unter dem Cursor hervorheben, passendes HTML-Tag hervorheben und mit umbenennen, Gehe zu Symbol (Strg+R), Navigationsverlauf (Alt+←/→), Lesezeichen, Textwerkzeuge, Suchen/Ersetzen in der Auswahl, Markierungen am rechten Rand, klickbare Pfadleiste (auch Netzlaufwerke werden jetzt korrekt geöffnet), Projektsuche mit Dateimustern und `.gitignore`, Projektsuche auf dem Server über SSH (abbrechbar), automatisch speichern, lokaler Verlauf, neues Fensterlayout mit Logo, Vergleich als Tab (Git-Markierungen funktionieren jetzt auch auf Netzlaufwerken), Git-Tab mit Commit, Pull/Push und Branches, Übertragungsliste (Neuverbindung nach Verbindungsabbruch repariert), gespeicherte Serverbefehle, Zen-Modus, Schriftart und Zeilenhöhe, Farbwähler für CSS-Farben, Bildvorschau, Minimap.

Bugrunde vom 19.10.: Rechtsklick auf den Projektordner, Startseite (Karte wächst mit, Listen leeren), ICO-Dateien, Bilder in der Vorschau, Zurück/Vor und schmale Scrollleisten in der Vorschau, Dateifilter ohne Einfrieren und zentriert, Schriftart-Dropdown, gleich große Einstellungsseiten.

KI-Agenten (19.10.): Claude Code, Codex (ChatGPT) und Gemini CLI über ACP im rechten Panel (Reiter „KI“): Verlauf mit Markdown, Plänen, Werkzeugaufrufen und Diffs, Rückfragen, Stop; Modell, Aufwand und Modus wählbar (pro Agent gemerkt); Dateien anhängen (Büroklammer, Rechtsklick in der Seitenleiste, auch vom Server); 5-Stunden- und Wochenlimit von Claude; Einstellungsseite „KI-Agenten“ mit Anmeldung über die Anbieter, API-Schlüsseln und Node.js-Verwaltung (Node wird bei Bedarf nach `%APPDATA%\nebula\runtime` geladen, Agenten liegen in `%APPDATA%\nebula\agents`); Änderungen des Agenten werden bei „Beim Speichern hochladen“ auf den Server übertragen. Außerdem: Reiter im rechten Panel jetzt Gliederung, KI, Vorschau.

Bugrunde vom 20.10.: Theme je für helles und dunkles Design mit Umschalter Hell/Dunkel/Automatisch (Einstellungen, Menü „Ansicht“, Knopf in der Titelleiste), Menüeintrag „KI-Agenten…“, Terminal-Knopf in der Titelleiste, geteilte Ansicht als Knopf in der (jetzt so hohen wie die Panel-Reiter) Pfadleiste, ID-Kollisionsmeldungen in den Einstellungen von Erweiterungen (z. B. Rust), Knopf für die Minimap neben dem für die geteilte Ansicht.

Serververwaltung (22.10.): Server in einklappbaren Gruppen, duplizieren, Import aus FileZilla (`sitemanager.xml`, Ordner als Gruppen, Passwörter auf Wunsch) und aus Nebula-Exporten, Export als JSON ohne Passwörter; Dubletten werden erkannt.

Schnell öffnen (22.10.): auch Dateien aus geladenen Serverordnern, `datei:Zeile[:Spalte]` und `:Zeile` für den aktuellen Tab; Treffer im Dateinamen werden besser gewichtet.
