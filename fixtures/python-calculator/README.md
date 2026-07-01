# Python Calculator Fixture

Tiny deterministic fixture for Mixmod experiments.

Run tests:

```sh
python -m unittest -q
```

Initial state has one failing test: `divide(1, 0)` returns `0` instead of raising `ValueError`.
