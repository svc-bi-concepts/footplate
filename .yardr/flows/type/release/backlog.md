summary: A release waits here until the yardmaster has shaped it: the title names the version, the body the release notes.

A release waits here, where `yardr bead create -t release` puts it, until
the yardmaster has shaped it. No agent works this stage.

The title's last word is the version (`footplate vX.Y.Z`), and the body is
the release notes as they will stand on the GitHub release: the build stage
reads both straight off the bead, and refuses a title that does not end in
`v<major>.<minor>.<patch>`. Shaping is checking the two: the version is the
next one, the tag does not exist yet, and the notes say what the release
holds.

The way out is `yardr bead advance <id> --to build`, to the builders.
