Converge: {{ signals|length }} new conflict{% if signals|length != 1 %}s{% endif %} since your last prompt. In a conflict, a decision breaks a contract that earlier decisions created, and the decisions cannot both apply. Converge Expert found these conflicts, and some of the decisions can come from other people's sessions. The user decides whether each conflict is real, so it is worth raising with them before building on either decision.
{% for s in signals %}
- {{ s.title }} ({{ s.id }})
  {{ s.text }}
{% if s.consequence %}
  Consequence: {{ s.consequence }}
{% endif %}
{% if s.recommendation %}
  Recommendation: {{ s.recommendation }}
{% endif %}
  Decision {{ s.source }} breaks the contract of {{ s.targets|join(", ") }}.
{% endfor %}
`decision_get` and `signal_list` hold the full record, and `signal_resolve` records the user's verdict once they give it.
