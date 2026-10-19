---
name: nebula-ki-scope
description: "Abgestimmter Umfang der KI-Integration in Nebula (Reihenfolge, Anbieter, Zusatzfunktionen)"
metadata:
  node_type: memory
  type: project
  originSessionId: 10a7e079-fb2f-49fa-8901-631ac611494f
  modified: 2026-10-19T13:23:00.039Z
---

Am 2026-10-19 mit dem Nutzer abgestimmt:
- Start mit Agenten über ACP (Agent Client Protocol, wie Zed): Chat im rechten Panel, Änderungen als Diff, Rückfragen vor Datei-/Befehlszugriff.
- Danach eigener KI-Chat mit Anbietern Anthropic, OpenAI (auch kompatible Basis-URL) und Ollama lokal; API-Schlüssel in der Windows-Anmeldeinformationsverwaltung.
- Ebenfalls gewünscht: KI-Befehle auf der Auswahl (Erklären, Kommentieren, Umschreiben, Fehler beheben → Diff), Commit-Nachricht im Git-Tab, Nebula als MCP-Server, Inline-Vorschläge – diese aber unbedingt ein-/ausschaltbar (Kosten).

**Why:** Nutzer hatte KI bewusst zurückgestellt und wollte den Umfang vorher festlegen.
**How to apply:** In dieser Reihenfolge abarbeiten, pro Teilschritt ein Commit (siehe [[nebula-workflow]], [[nebula-progress]]).

Stand 2026-10-19: ACP-Kern (src/acp/, Typen aus Crate agent-client-protocol-schema ohne async) und Agent-Reiter im rechten Panel (src/app/agent.rs, Strg+Umschalt+A) fertig und mit Schein-Agent getestet (Node-Skript im Scratchpad, Tests in src/acp/tests.rs). Einstellungsseite „KI-Agenten“ (src/app/agents_page.rs) fertig: Login macht der Agent selbst (Claude: Terminal-Methode → Konsolenfenster, Codex „chat-gpt“/Gemini „oauth-personal“: authenticate öffnet Browser), API-Schlüssel per Keyring als Umgebungsvariable. „Status prüfen“ (initialize+session/new) kostet kein Kontingent; Prompts an echte Agenten nur nach Rückfrage. Codex meldet auch ohne Login „bereit“. Offen: echter Prompt-Test, ggf. Terminal-Fähigkeit; danach eigener Chat.

