This is the HOME volume: the persistent disk, kept apart from the system image so the system can be rebuilt without losing
what is written here.

This file came from a stage's disk-home-seed/ folder, which `just home-disk` copies into a new home.img (at the repository root, shared by every stage) once. That folder is only a
starting point: it is not kept in step with home.img, and nothing written to home.img ever goes back to it.
