## Converge memory — project "{{ project.name }}" ({{ project.id }})
{% if not decisions %}
This working tree is bound to converge project `{{ project.id }}`; project memory is active. No decisions are recorded yet — use `decision_add` when a design decision lands. A hook attaches the exchange that decided it, and completes a bare `path:lines` into a code citation.
{% else %}
This working tree is bound to converge project `{{ project.id }}`; project memory is active. Decisions below are in force — `decision_get` for the full record before re-deciding a settled topic; `decision_add` (with `supersedes` when it replaces one) when a new decision lands. A hook attaches the exchange that decided it, and completes a bare `path:lines` into a code citation.

Decisions{% if new_decisions %} (← NEW = not shown to you before, in any session){% endif %}:
{% for d in decisions %}
- {{ d.title }} [{{ d.status }}]{% if d.new %} ← NEW{% endif %}

{% endfor %}
{% endif %}
{% if signals %}

Proposed signals (unjudged observations touching this project{% if new_signals %}; ← NEW = not shown to you before, in any session{% endif %} — raise conflict-tier ones with the user proactively; `signal_list` for the full record, then `signal_resolve` with THEIR verdict, never your own):
{% for s in signals %}
- [{{ s.tier }}/{{ s.kind }}] {{ s.title }} ({{ s.id }}){% if s.new %} ← NEW{% endif %}

{% endfor %}
{% endif %}
