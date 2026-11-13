# plugin-project-model-yarn-workspaces

Basalt project-model plugin for Yarn Workspaces / Lerna monorepos.

Activates on `package.json` and `lerna.json`. Emits the generic
Basalt project-model JSON for each workspace package.

## Provides
- `project-model:yarn-workspaces`

## Activation globs
- `package.json`, `**/package.json`, `lerna.json`, `**/lerna.json`
