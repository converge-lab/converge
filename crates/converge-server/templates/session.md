## Converge memory — project "{{ project.name }}" ({{ project.id }})
{% if not decisions %}
This working tree is bound to converge project `{{ project.id }}`. No decisions are recorded yet; when the user settles something others will need to know, `decision_add` keeps it for the team. A hook attaches the exchange that decided it, and completes a bare `path:lines` into a code citation.
{% else %}
This working tree is bound to converge project `{{ project.id }}`. The decisions below are the team's answers so far: `decision_get` shows why one was made, worth reading before going against it, and `decision_add` (with `supersedes` if it replaces one) keeps what the user settles next. A hook attaches the exchange that decided it, and completes a bare `path:lines` into a code citation.

Decisions{% if new_decisions %} (← NEW = not shown to you before, in any session){% endif %}:
{% for d in decisions %}
- {{ d.title }} [{{ d.status }}]{% if d.new %} ← NEW{% endif %}

{% endfor %}
{% endif %}
{% if signals %}

Proposed signals (observations about this project's decisions that nobody has judged yet{% if new_signals %}; ← NEW = not shown to you before, in any session{% endif %}). A conflict means two decisions can't both stand, so it is worth raising with the user before building on either. Whether a signal holds is their call, recorded with `signal_resolve`; `signal_list` has the full text:
{% for s in signals %}
- [{{ s.tier }}/{{ s.kind }}] {{ s.title }} ({{ s.id }}){% if s.new %} ← NEW{% endif %}

{% endfor %}
{% endif %}
