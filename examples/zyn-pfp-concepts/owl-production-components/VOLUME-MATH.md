# Owl component volume math

Collection 01 contains 889 Moths and 1,333 Owls.

```text
Owl / Moth supply ratio = 1,333 / 889 = 1.4994376 ≈ 1.5
Moth visual masters      = 12 + 10 + 7 + 10 + 9 + 12 = 60
Owl visual-master target = 60 × 1.5 = 90
```

The 90-master Owl inventory is:

| Category | Moth | Owl | Scaling decision |
|---|---:|---:|---|
| Background | 12 | 18 | exact 1.5× |
| Body | 10 | 15 | exact 1.5× |
| Mouth / beak | 7 | 11 | 10.5 rounded up |
| Eyes | 10 | 15 | exact 1.5× |
| Headgear | 9 | 13 | 13.5 rounded down |
| Light previews | 12 | 18 | exact 1.5×; includes neutral |
| Total | 60 | 90 | exact 1.5× |

Rounding Mouth upward and Headgear downward keeps the total exact while giving
the Owl more species-specific facial expression range. The 18 light previews are
neutral, the 11 canonical collection lights and six Owl-specific visual
subvariants. The subvariants can map back to canonical Light metadata values
unless the global light taxonomy is expanded later.

For the later Mole pack, the same rule gives `2,222 / 889 ≈ 2.5`, or a target of
150 visual masters relative to the Moth's 60.
