# Mole component volume math

Collection 01 contains 889 Moths, 1,333 Owls and 2,222 Moles.

```text
Mole / Moth supply ratio = 2,222 / 889 = 2.4994376 ≈ 2.5
Moth visual masters       = 12 + 10 + 7 + 10 + 9 + 12 = 60
Mole visual-master target = 60 × 2.5 = 150
```

| Category | Moth | Mole | Scaling decision |
|---|---:|---:|---|
| Background | 12 | 30 | exact 2.5× |
| Body | 10 | 25 | exact 2.5× |
| Mouth | 7 | 18 | 17.5 rounded up |
| Eyes | 10 | 25 | exact 2.5× |
| Headgear | 9 | 22 | 22.5 rounded down |
| Light previews | 12 | 30 | exact 2.5×; includes neutral |
| Total | 60 | 150 | exact 2.5× |

Rounding Mouth upward and Headgear downward preserves the exact target while
giving the Mole more species-specific expression range. The 30 light previews
contain neutral, the 11 canonical collection lights and 18 Mole-specific visual
subvariants that may map back to canonical Light metadata values.
