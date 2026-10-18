# Nebula – offene Ideen

Priorität: **A** = fehlt spürbar im Alltag, **B** = deutlicher Komfortgewinn, **C** = nice to have.

## Sicherheit und Datenerhalt (fehlt bisher ganz)

- **A** Externe Änderungen erkennen: Lokale Datei wurde außerhalb von Nebula geändert (Git-Checkout, anderes Programm) → ohne eigene Änderungen still neu laden, sonst nachfragen. Auch gelöschte/umbenannte Dateien melden. Seitenleiste automatisch aktualisieren (Dateisystem-Überwachung statt „Aktualisieren“-Knopf).
- **A** Wiederherstellung nach Absturz („Hot Exit“): Ungespeicherte Inhalte (auch unbenannte Tabs) regelmäßig sichern und beim nächsten Start wiederherstellen.
- **B** Automatisches Speichern (optional: bei Fokusverlust oder nach x Sekunden), für Serverdateien eher aus.
- **B** Lokaler Verlauf: frühere Fassungen einer Datei (z. B. die letzten 20 Speicherstände) ansehen, vergleichen und wiederherstellen – besonders wertvoll beim direkten Arbeiten auf dem Server.
- **B** Backup vor dem Überschreiben auf dem Server (optional `.bak` oder lokale Kopie).

## Editor

- **A** Geteilte Ansicht: zwei Dateien (oder dieselbe Datei an zwei Stellen) nebeneinander/untereinander.
- **A** Block-/Spaltenauswahl mit Alt+Ziehen (Rechteckauswahl), dazu „Cursor an alle Zeilenenden der Auswahl“ (Alt+Umschalt+I).
- **A** Formatieren ohne Erweiterung: Leerzeichen am Zeilenende entfernen, Zeilenumbruch am Dateiende sicherstellen (beides als Option beim Speichern), Einrückung Tabs ↔ Leerzeichen umwandeln.
- **A** `.editorconfig` auswerten (Einrückung, Zeilenenden, Kodierung pro Projekt).
- **B** Minimap bzw. Markierungen in der Scrollleiste (Suchtreffer, Fehler, geänderte Zeilen, Cursor).
- **B** Textwerkzeuge: Zeilen sortieren, Duplikate entfernen, GROSS/klein/Titel, Zeilen verbinden, Auswahl umschließen (Klammern/Anführungszeichen/Tag um markierten Text), Zeilen umkehren.
- **B** Lesezeichen (Strg+F2 setzen, F2 springen) mit Markierung in der Leiste.
- **B** Gehe zu Symbol im Dokument (Liste aller Funktionen/Klassen/Selektoren/Überschriften) – die Pfadleiste kennt die Symbole schon.
- **B** Klickbare Pfadleiste: Klick auf ein Element zeigt Geschwister zum Springen (wie Nova).
- **B** Passendes HTML-Tag hervorheben und beim Umbenennen des öffnenden Tags das schließende mit ändern.
- **B** Wort unter dem Cursor überall im Dokument dezent hervorheben.
- **B** Eigene Snippets pro Sprache in den Einstellungen anlegen/bearbeiten (mit Platzhaltern `$1`, `${2:name}`).
- **C** Makros aufnehmen und abspielen.
- **C** Rechtschreibprüfung für Texte, Kommentare und Markdown (Windows-Rechtschreibprüfung nutzen).
- **C** Zen-/Fokusmodus (nur Editor, Vollbild).
- **C** Schriftart wählbar (inkl. Ligaturen), Zeilenhöhe einstellbar.

## Suchen und Navigieren

- **A** Zuletzt geöffnete Ordner/Projekte (nicht nur Dateien) und schnelles Wechseln zwischen Projekten.
- **B** Projektsuche mit Ein-/Ausschlussmustern (`*.php`, `!vendor/**`) und Berücksichtigung der `.gitignore`.
- **B** Suchen/Ersetzen nur in der Auswahl; Regex-Ersetzung mit Vorschau.
- **B** Navigationsverlauf: Zurück/Vor zur letzten Cursorposition (Alt+←/→), auch über Dateien hinweg.
- **B** „Schnell öffnen“ (Strg+P) auch für Dateien auf dem Server (aus bereits geladenen Ordnern) und mit `:Zeile`-Suffix.
- **C** Gehe zu Definition ohne Language Server für einfache Fälle (CSS-Klassen, PHP-Funktionen im Projekt).

## Git

- **A** Geänderte Zeilen in der Zeilennummernleiste markieren (neu/geändert/gelöscht) – Branch und Dateistatus sind schon da.
- **B** Diff-Ansicht: Arbeitsstand gegen letzten Commit, und allgemein zwei Dateien vergleichen (auch lokal ↔ Server).
- **B** Einfache Git-Aktionen: Dateien stagen, Commit mit Nachricht, Pull/Push, Branch wechseln.
- **C** Blame für die aktuelle Zeile (Autor/Datum dezent am Zeilenende).

## Server und Veröffentlichen

- **A** Konfliktdialog um „Unterschiede anzeigen“ erweitern: Diff lokal ↔ Server, bevor überschrieben wird.
- **B** Synchronisieren in beide Richtungen mit Vorschau (neu/geändert/gelöscht) und Ausschlussliste (`.git`, `node_modules`, `.env`).
- **B** Übertragungsliste: laufende/fertige/fehlgeschlagene Übertragungen mit Fortschritt, Abbrechen und Wiederholen.
- **B** Server in Ordnern gruppieren, Server duplizieren, Import aus FileZilla (`sitemanager.xml`).
- **B** SSH-Terminal: gespeicherte Befehle pro Server per Klick (Cache leeren, `git pull`, Deploy), Suche im Terminal-Verlauf, lokales Terminal (PowerShell) im selben Panel.
- **C** Datenbank light: MySQL über SSH-Tunnel, Tabellen ansehen, Abfragen ausführen.
- **C** Serverdateien zwischenspeichern, damit sie offline lesbar bleiben.

## Live-Vorschau und Web

- **B** Vorschau in verschiedenen Gerätebreiten (Handy/Tablet/Desktop) umschaltbar.
- **B** Lokaler PHP-Server für die Vorschau (wenn `php.exe` vorhanden), statt nur Live-URL.
- **C** Farbwähler beim Klick auf die Farbvorschau unter CSS-Farbwerten.
- **C** Bildvorschau: Bilder (PNG, JPG, SVG, WebP) in einem Tab anzeigen statt „keine Textdatei“, mit Größe/Abmessungen.

## Oberfläche und Allgemein

- **A** Eigene Tastenkürzel festlegen (Tastenbelegung in den Einstellungen, Konflikte anzeigen).
- **B** Tabs anheften; mehrere Fenster/Projekte gleichzeitig.
- **B** Große Dateien (> 64 MB) schreibgeschützt öffnen statt ablehnen; Hex-Ansicht für Binärdateien.
- **B** Einstellungen exportieren/importieren (Server ohne Passwörter, Themes, Tastenkürzel).
- **B** Update-Hinweis, Installer und Eintrag „Mit Nebula öffnen“ im Explorer-Kontextmenü, Dateizuordnungen.
- **C** Oberfläche auch auf Englisch (Texte in Übersetzungsdateien auslagern).
- **C** Barrierefreiheit: Bildschirmleser im Editor (AccessKit ist schon eingebunden), hoher Kontrast.

## Qualität und Technik

- **B** Performance mit sehr großen Dateien (10–50 MB, minifiziertes JS in einer Zeile) und vielen Tabs prüfen; Highlighting inkrementell statt komplett neu.
- **B** Absturzbericht: Panics in eine Logdatei schreiben und beim nächsten Start anbieten.
- **C** Automatisierte UI-Tests für die wichtigsten Abläufe (Öffnen, Bearbeiten, Speichern lokal/Server).
