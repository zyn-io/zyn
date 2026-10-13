A host's own modules: a gate (`Den.Gate`) or processes an add-on starts, in
`extra/lib`, and their tests in `extra/test`. The den compiles them in when
`extra/lib` is there (mix.exs) and the image copies this folder; neither is
part of the den itself, and both are ignored by git.
