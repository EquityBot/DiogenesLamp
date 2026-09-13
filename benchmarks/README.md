# Viewer performance baseline

The numbers below measure the pure catalogue path over 10,000 fabricated albums: duplicate identity, multi-term filtering, global sorting and duplicate collapse. Each run took 30 post-warm-up samples from Node 22's stable `perf_hooks` clock, and reported median, standard deviation and coefficient of variation. There is no pass/fail duration threshold and nothing here runs in CI.

## Baseline recorded 2026-08-28

Five independent Node v22.23.2 runs on the same Windows workstation each used
30 post-warm-up samples. Every sample returned the stable result `5000:5000`.

| Run | Mean | Median | Within-run CV |
| --- | ---: | ---: | ---: |
| 1 | 13.084 ms | 12.971 ms | 5.47% |
| 2 | 13.804 ms | 13.562 ms | 6.71% |
| 3 | 13.772 ms | 13.539 ms | 6.28% |
| 4 | 13.415 ms | 13.188 ms | 8.51% |
| 5 | 13.323 ms | 13.110 ms | 6.19% |

Across the 5 run means, the mean was 13.479 ms, the population standard
deviation was 0.274 ms and the coefficient of variation was 2.03%. The medians
ranged from 12.971 to 13.562 ms. This is sufficiently repeatable to retain as a
comparison baseline, but it does not yet justify a regression threshold across
different hosts.
