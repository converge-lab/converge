Converge: {{ signals|length }} signal{% if signals|length != 1 %}s{% endif %} raised since your last prompt — an expert model's observations about decisions in this project, some possibly from other people's sessions. They are information for the user, who decides whether each holds.{% if conflicts %} A conflict means two decisions can't both stand, so it is worth raising before building on either.{% endif %}

{% for s in signals %}
- [{{ s.tier }}/{{ s.kind }}] {{ s.title }} ({{ s.id }}): {{ s.text }}
{% if s.recommendation %}
  Recommendation: {{ s.recommendation }}
{% endif %}
{% endfor %}
The user will want to hear about these; `decision_get` and `signal_list` hold the full record, and `signal_resolve` records the user's verdict once they give it.
