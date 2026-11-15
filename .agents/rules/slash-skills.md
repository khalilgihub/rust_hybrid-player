---
trigger: always_on
---

## Slash Command Skill Invocation

Whenever the user starts their prompt with a slash followed by a skill name, or uses `/skill <name>`:
- `/frontend-design [task]` -> Immediately activate and execute the `frontend-design` skill located at `.agents/skills/frontend-design/SKILL.md`. Follow its aesthetic direction, typography, and anti-generic design principles.
- `/ui-ux-pro-max [task]` -> Immediately activate and execute the `ui-ux-pro-max` skill located at `.agents/skills/ui-ux-pro-max/SKILL.md`. Query its local design tokens, palettes, font pairings, and UX guidelines.
- `/web-artifacts-builder [task]` -> Immediately activate and execute the `web-artifacts-builder` skill located at `.agents/skills/web-artifacts-builder/SKILL.md`. Run its artifact initialization, development, and bundling workflows.
- `/skill <name> [task]` -> Immediately activate and execute the specified skill from `.agents/skills/<name>/SKILL.md`.

### Execution Protocol:
1. Read the corresponding `SKILL.md` file using `view_file` to load its full procedure.
2. Apply the skill's specific methodology, heuristics, and scripts to the user's request.
