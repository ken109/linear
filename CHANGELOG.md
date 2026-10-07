# Changelog

## [0.6.0](https://github.com/ken109/linear/compare/v0.5.0...v0.6.0) (2026-10-07)


### ⚠ BREAKING CHANGES

* **core:** select priority on every issue

### Features

* **cli:** a dry run with --force reports the ownership override it would make ([e726040](https://github.com/ken109/linear/commit/e7260403ceeb2aa50ff7d23d03455a038de0a1f6))
* **cli:** add --force to the writes the ownership rules guard ([4795737](https://github.com/ken109/linear/commit/479573714b4b5d78fe08745c8ec238ac8f894201))
* **cli:** add `comment update` and `comment delete` ([62eb30f](https://github.com/ken109/linear/commit/62eb30f418c13d794a8b1d6ce9070de180158f70))
* **cli:** add `issue relate` and `issue unrelate`, show relations in `issue view` ([30de477](https://github.com/ken109/linear/commit/30de4777035ad08c0a87c206ebae9436a177e537))
* **cli:** add `linear completions <shell>` for bash, zsh, fish, elvish and powershell ([1aae9a8](https://github.com/ken109/linear/commit/1aae9a86b8e5a6d01cb384c69143ef89a23a4b5b))
* **cli:** add `linear webhook list|create|delete|verify` ([f8ec45a](https://github.com/ken109/linear/commit/f8ec45a25161d97b8a4d852e54904266190be6f4))
* **cli:** add cycle list and cycle view ([3729cdc](https://github.com/ken109/linear/commit/3729cdca89d4cf7db112ac86c5846f20c31c4706))
* **cli:** add document list, view, create and update ([d966c6f](https://github.com/ken109/linear/commit/d966c6f4c3bb9f4f4dddb6469ae4dda943e4d849))
* **cli:** add issue batch, creating and updating several issues all or nothing ([c2c5a1d](https://github.com/ken109/linear/commit/c2c5a1d97fc29565fa29cb92a41461369c0037a5))
* **cli:** add label create and label update ([fc92650](https://github.com/ken109/linear/commit/fc92650fd012c2be6f37034e6f4740b02101c737))
* **cli:** add linear usage and linear &lt;group&gt; usage, generated from the command tree ([32d6138](https://github.com/ken109/linear/commit/32d6138028602eb88d2f228e5a420f505b7e3532))
* **cli:** add the --dry-run flag and the plan every write prints ([57348db](https://github.com/ken109/linear/commit/57348dba4db309aba59d40ffae59995026c7b35e))
* **cli:** cut list and view output down with --fields and --id-only ([886c2ca](https://github.com/ken109/linear/commit/886c2ca3bc88eafc59e3f31d23a750ac3ab03ea8))
* **cli:** dry-run the issue, comment and file writes ([b879707](https://github.com/ken109/linear/commit/b879707632934ed112768f451f7673619583889b))
* **cli:** dry-run the milestone, initiative, template, label and document writes ([a277d3f](https://github.com/ken109/linear/commit/a277d3f21cc9d74e6fafc088c103e5017d20bcd1))
* **cli:** dry-run the project writes ([71dcc90](https://github.com/ken109/linear/commit/71dcc901cff3de2712c6a07bc00f7f816d535576))
* **cli:** dry-run webhook create and delete and raw mutations ([1992601](https://github.com/ken109/linear/commit/19926018a761ced24dfd0ec2079b5cf7b5737c05))
* **cli:** file upload, file download and issue attach-file ([b790f78](https://github.com/ken109/linear/commit/b790f78e99ff71f72c9b4c30209739ceb7732108))
* **cli:** initiative archive, unarchive and delete ([94a0da6](https://github.com/ken109/linear/commit/94a0da650227dfb2da275b7649de0f95635f242b))
* **cli:** initiative update, add-project, remove-project and status updates ([b16f2d9](https://github.com/ken109/linear/commit/b16f2d94ebac75589fae09a4123c5bd4fbcf245c))
* **cli:** issue search through Linear's searchIssues ([44d152b](https://github.com/ken109/linear/commit/44d152bc7e7acf35c82f8dd368f6e0ee917d271b))
* **cli:** issue unlink, delete, archive and unarchive ([dee63bd](https://github.com/ken109/linear/commit/dee63bd7c3314a7d5227d0ad7c80432cb860b3f6))
* **cli:** keep credentials in the OS keyring, with the file as a fallback ([781de8d](https://github.com/ken109/linear/commit/781de8d3641403c5b5c996c494bf9c8479e93b4a))
* **cli:** log in with OAuth (PKCE) and refresh the token ([b3f754b](https://github.com/ken109/linear/commit/b3f754b685ba9c9c258d1d451e9c266cc65375fb))
* **cli:** project delete and unarchive ([e446b88](https://github.com/ken109/linear/commit/e446b88fe1abdadc6211807a3b1165541b001118))
* **cli:** set and filter issues by priority, estimate, parent and cycle ([c6cdb8e](https://github.com/ken109/linear/commit/c6cdb8e30cf30badb2abd7511137635adea7663f))
* **cli:** show allow_force in workspace list ([b581381](https://github.com/ken109/linear/commit/b581381d03750846473321f5408c3b4bca0c539a))
* **core:** add the credential_store workspace setting ([d4b91cf](https://github.com/ken109/linear/commit/d4b91cf6779fe3badc096e647b875458f317a10b))
* **core:** add the pure half of the OAuth PKCE login ([0e64fc4](https://github.com/ken109/linear/commit/0e64fc400da590ab32b7948b9b6d79c4bfcfe345))
* **core:** file upload operations, size limits and markdown embedding ([b6381ef](https://github.com/ken109/linear/commit/b6381ef4239eb47894c8ab3c0a6cdb9911a38c75))
* **core:** initiative update, project link removal and status updates ([d800351](https://github.com/ken109/linear/commit/d8003513e1f03bfee27d62f35a1ab5077b56dc97))
* **core:** let the ownership guard report an overridden refusal ([4592cca](https://github.com/ken109/linear/commit/4592cca106769412c0fa971f2c35764832fc5a7a))
* **core:** list every error code in ErrorCode::ALL ([4d27321](https://github.com/ken109/linear/commit/4d27321d03ffc132b34e101d42f6d98d81c43743))
* **core:** query a team's cycles with their state and the issues in one ([aa862ab](https://github.com/ken109/linear/commit/aa862ab1db3569178f619e990b3a9726bb1c554d))
* **core:** query, create and delete webhooks, and match one by id, label or URL ([a75a0d2](https://github.com/ken109/linear/commit/a75a0d24412c5ee0eb90fbb95bb3ecc920d42c06))
* **core:** read and write documents, and hold their bodies to document templates ([25fa244](https://github.com/ken109/linear/commit/25fa244d415c10613ec0e362bf5313ef7e671bcb))
* **core:** read and write labels with their team and group, and check regrouping against issues ([b029415](https://github.com/ken109/linear/commit/b0294156639cb5d789a3736ab581b36f098606e6))
* **core:** select priority on every issue ([be6de5d](https://github.com/ken109/linear/commit/be6de5d711d17e42b1f281f65d6e264b776cd54c))
* **core:** the input of issue batch and its JSON Schema ([8c46591](https://github.com/ken109/linear/commit/8c46591b6f578ed0b38e742e60fa2335c12d60f8))
* **core:** wire comment update/delete and issue relation create/delete ([a42bc68](https://github.com/ken109/linear/commit/a42bc68d450b6ea018608a3cbc250ec74d8f6920))


### Bug Fixes

* **cli:** keep the config in %APPDATA% and the cache in %LOCALAPPDATA% on Windows ([283dd02](https://github.com/ken109/linear/commit/283dd0297a9cf21567d3d5c4ba13cf4a134a27bc))
* **cli:** refuse --keyring for an app workspace and ignore an env API key for an oauth one ([298c49f](https://github.com/ken109/linear/commit/298c49fd30d902a7cd6b505558d206f6c0a5786f))


### Code Refactoring

* **cli:** keep an undo with the mutation it undoes, and let create and update run in an open session ([51fe9e7](https://github.com/ken109/linear/commit/51fe9e7e229d48dca359069e7278585d5aebe256))


### Documentation

* delete and archive commands in the README ([e795c81](https://github.com/ken109/linear/commit/e795c81ae93fc8cb92a28222139919a4cd54c874))
* describe --force and allow_force ([6dd6caa](https://github.com/ken109/linear/commit/6dd6caa349efa41252f20470f37259a0bdd79482))
* document comment update/delete and issue relate/unrelate ([aec88e8](https://github.com/ken109/linear/commit/aec88e8450b2138a37da06cf09ee0b11dca9235b))
* initiative updates, project links, status updates and files in the README ([bcdc0d8](https://github.com/ken109/linear/commit/bcdc0d804f8120c40526e1842e72ef16aa8ac4ac))
* **readme:** document --dry-run, and check it against the sandbox ([ce51dd0](https://github.com/ken109/linear/commit/ce51dd0af886196e2dcb1f883440e8433c6ed1af))
* **readme:** document issue batch ([5ea0aca](https://github.com/ken109/linear/commit/5ea0aca9d0b1d63c14487c82ee9714ab2e2dd5b7))
* **readme:** document the OAuth login, its callback port and the refresh ([6cbd2dc](https://github.com/ken109/linear/commit/6cbd2dc00c448c6e0ee60e0070c5e074aa1a6ce4))
* **readme:** document the OS keyring credential store ([5bc06a1](https://github.com/ken109/linear/commit/5bc06a181d7d4356285c43deb4a97548e96972fc))
* **readme:** say how --dry-run and --force compose ([396b57b](https://github.com/ken109/linear/commit/396b57bb92852b7013f9dfacafbb4bf7e3c80445))
* say the duplicate state may take a few seconds to clear ([9c9a585](https://github.com/ken109/linear/commit/9c9a5856ca8e9fb19b7a70b3539b2ca1feafc106))
* shell completions and installing on Windows ([0b199e7](https://github.com/ken109/linear/commit/0b199e754d38b87912dcbda58dc7c87d7df8622d))
* ship a Claude Code skill and a README section for AI agents ([cd30c01](https://github.com/ken109/linear/commit/cd30c01419d7e7d35f0e106d4536f3f9d4b1419e))
* the webhook commands ([bc88e04](https://github.com/ken109/linear/commit/bc88e04dedfbd2fa6d4b1f9c7a8b435f6ce5b14b))

## [0.5.0](https://github.com/ken109/linear/compare/v0.4.0...v0.5.0) (2026-10-07)


### ⚠ BREAKING CHANGES

* **cli:** camelCase the remaining snake_case keys of --json output

### Features

* **cli:** camelCase the remaining snake_case keys of --json output ([8afa1f6](https://github.com/ken109/linear/commit/8afa1f64c5faa0fdad6e04c9cfa0e794e1b52a75))

## [0.4.0](https://github.com/ken109/linear/compare/v0.3.0...v0.4.0) (2026-10-07)


### ⚠ BREAKING CHANGES

* **core:** read GitHub pull requests off attachments and select branchName

### Features

* **cli:** issue link-pr, and branchName and pullRequests in issue view ([9d58eba](https://github.com/ken109/linear/commit/9d58eba663941b9f66e19879fe51c9ff53984883))
* **core:** audit rules for merged and long-open GitHub pull requests ([aac7125](https://github.com/ken109/linear/commit/aac71254076931fb885c670f3cd100573d053509))
* **core:** read GitHub pull requests off attachments and select branchName ([e769904](https://github.com/ken109/linear/commit/e769904c89f9c76d5ae099e4d0669cc32334f8f4))

## [0.3.0](https://github.com/ken109/linear/compare/v0.2.0...v0.3.0) (2026-10-07)


### ⚠ BREAKING CHANGES

* **core:** camelCase JSON keys in the audit and cache shapes

### Features

* **core:** camelCase JSON keys in the audit and cache shapes ([95bd336](https://github.com/ken109/linear/commit/95bd336adae308b973c96c879cba50c5e408446e))
* **core:** require a template to replace a body under template-sections ([f2c3577](https://github.com/ken109/linear/commit/f2c3577a63b566b3e7d8cfb5a8e1d7a612d45d76))


### Code Refactoring

* **cli:** decide a cache refresh with core's decide_refresh ([73c1812](https://github.com/ken109/linear/commit/73c18125cd36e0aaf46addcb68d064a82dc000ce))


### Documentation

* say template-sections requires --template on body updates ([7610a01](https://github.com/ken109/linear/commit/7610a01317ff39cf0e1a02a0d9d5617d751291cb))

## [0.2.0](https://github.com/ken109/linear/compare/v0.1.0...v0.2.0) (2026-10-07)


### Features

* **template:** create project templates ([f66449f](https://github.com/ken109/linear/commit/f66449f86abfd570bd917aefb3d2920026921178))


### Documentation

* note how template-sections reaches project bodies and that off-calendar dates are refused ([ff13610](https://github.com/ken109/linear/commit/ff136107b55d0f02ac21710ddcf1db95c7d327ad))
* stop pointing at the retired tool in README and comments ([2d0d66e](https://github.com/ken109/linear/commit/2d0d66e3f9d3c25704304b572d4c52c72de00490))
* **template:** document project templates and the --type flag ([c3beb03](https://github.com/ken109/linear/commit/c3beb03660fc375d8fb196fd400a7a7bebd807e5))

## [0.1.0](https://github.com/ken109/linear/compare/v0.1.0...v0.1.0) (2026-10-07)


### Features

* **auth:** authenticate as an app with the client credentials grant ([d81b637](https://github.com/ken109/linear/commit/d81b637ba61a23b0c4ad9a3b4a1402e7f3fe28f6))
* **cli:** add `linear brief` and `linear brief --session` ([a6ff7e1](https://github.com/ken109/linear/commit/a6ff7e10971388cca3cfb29cc8be44dac2f940a5))
* **cli:** add `linear cycle` to print the cycle for a meeting day ([e23f0b7](https://github.com/ken109/linear/commit/e23f0b708a543c128409691a83c411f689238ef7))
* **cli:** add cache refresh, show and clear ([a5e93bf](https://github.com/ken109/linear/commit/a5e93bff9bcbef4bb0cbf424b2079919d8b5e5e4))
* **cli:** add issue create --meta for source attachment metadata ([2ad7daf](https://github.com/ken109/linear/commit/2ad7daf5cf8ae21d900d6d3ba30c4848fceb5d26))
* **cli:** add issue create, update, comment and reorder on a shared write path ([70ef115](https://github.com/ken109/linear/commit/70ef115d9c917b1a523c1301e2075e9e50986a55))
* **cli:** add issue list and view ([376e806](https://github.com/ken109/linear/commit/376e806dc9448f1c1f8127dfc7c08154253ac0d9))
* **cli:** add linear api for raw read-only GraphQL ([8519108](https://github.com/ken109/linear/commit/8519108ec17f63d14d9708f06ac3626f328b3d29))
* **cli:** add milestone create, update and delete, initiative create and template create ([7361d97](https://github.com/ken109/linear/commit/7361d97858d4018d5244604ee40301d0edfc4e0d))
* **cli:** add milestone, initiative, template, label, team and user read commands ([86cdbb2](https://github.com/ken109/linear/commit/86cdbb26686b9dd77cec5104b62eb9661d5cf595))
* **cli:** add output conventions, exit codes and workspace list ([afb0ec6](https://github.com/ken109/linear/commit/afb0ec64aecad751e24556063080d8ebe386de32))
* **cli:** add project create, update, reorder and status-update ([e2dda83](https://github.com/ken109/linear/commit/e2dda83f9d47c5482ade13c93e6e6349262f04b4))
* **cli:** add project list and view ([1a5dff1](https://github.com/ken109/linear/commit/1a5dff1f34744fabb9aa7eaa8c1e025cc36fd59e))
* **cli:** add status, the cache in one line for a statusline ([fcfcc0d](https://github.com/ken109/linear/commit/fcfcc0d94a809ec756df09bf43a9f44c72da287e))
* **cli:** add the audit command ([ef5011f](https://github.com/ken109/linear/commit/ef5011f19011cb6ebd1f4073d997a8759a9302e8))
* **cli:** add workspace add, login and whoami with credential storage ([5b2ed7f](https://github.com/ken109/linear/commit/5b2ed7ff6d53ba05a8764e40094ab7c29678b378))
* **cli:** filter issue list by completion time and order it by sortOrder ([2044b39](https://github.com/ken109/linear/commit/2044b392ca80b37a45164f6c131ac853fddd7fc4))
* **cli:** put an issue created with --held-on in the cycle after the meeting ([d3730e3](https://github.com/ken109/linear/commit/d3730e39549bd6aa4cfd75eca11512dd89a430a2))
* **cli:** read issues and projects from the cache with --cached ([d02a523](https://github.com/ken109/linear/commit/d02a523d169c0eb290b8446d214be0cf5e59a13f))
* **cli:** send raw mutations from linear api when the workspace allows it ([cfcb70a](https://github.com/ken109/linear/commit/cfcb70a9cc5ccc664f768e50ea23195362375b0f))
* **cli:** update an issue's description, source and labels ([a577e85](https://github.com/ken109/linear/commit/a577e85da4cc3b0470483dad01e89271a2767d11))
* **core:** add audit consistency rules ([6e0469d](https://github.com/ken109/linear/commit/6e0469da955de01e264ce10599ca855ce87f8151))
* **core:** add audit diff for newly appeared findings ([e2131d8](https://github.com/ken109/linear/commit/e2131d8aff8a30e8a11dc692e60d085dba6f183c))
* **core:** add credential types with redacted secrets ([46084f5](https://github.com/ken109/linear/commit/46084f59ece6dc85146e2b58aeae4250b67c7a89))
* **core:** add declarative validator rule engine ([bae4fde](https://github.com/ken109/linear/commit/bae4fde1a4b6a0ce0cd61b05c143b7c305001060))
* **core:** add domain types as cynic fragments and typed queries ([5f865e5](https://github.com/ken109/linear/commit/5f865e577f389247a86f4164478d62aa0325eb86))
* **core:** add issue write inputs, write-context queries and the reorder plan ([540d977](https://github.com/ken109/linear/commit/540d9772ed6c292d3d075b856d9b9e39e0cb7f2c))
* **core:** add milestone, initiative and template write inputs ([730e926](https://github.com/ken109/linear/commit/730e926ad5f27a2095f216af13590b0ec44abd00))
* **core:** add ownership-based write guard ([c208462](https://github.com/ken109/linear/commit/c208462eeaed5c0ce0acb586e483e39d2a746a36))
* **core:** add project write inputs, mutations and the queries a write reads ([b16c540](https://github.com/ken109/linear/commit/b16c5406586c95b02635a58b9af655aa48c16bb3))
* **core:** add read queries, list filters and reference matching ([701cf1d](https://github.com/ken109/linear/commit/701cf1d1dd355c67a94f9154eff6b9649af68b37))
* **core:** add request building, response parsing and pagination ([4524500](https://github.com/ken109/linear/commit/4524500e2f0bbdbaa47b5e8057f222d1fe65af25))
* **core:** add stale-in-progress and status-update-outdated audit rules ([2a4310c](https://github.com/ken109/linear/commit/2a4310c069111af4d40674393e5828d23e127ccc))
* **core:** add the cache entry and its refresh rules ([41752b3](https://github.com/ken109/linear/commit/41752b3c8b87d2566c5685c5d9bb1ad44e9144ce))
* **core:** add workspaces.toml model and workspace resolution ([d7c7322](https://github.com/ken109/linear/commit/d7c7322819ca809bb72c71e6bdd677e26efb0988))
* **core:** audit template-sections on existing issues and read audit settings ([a997a89](https://github.com/ken109/linear/commit/a997a89b6fe347e3a981aa48a0c55961e49a0abb))
* **core:** build the brief of unfinished projects ([412cd71](https://github.com/ken109/linear/commit/412cd711ac67c7b9e87318018559a20a25e9451e))
* **core:** decide whether a cached snapshot should be refreshed ([9c019b0](https://github.com/ken109/linear/commit/9c019b0c3c0034961aca0c832a58a6bcdbead2ff))
* **core:** describe AttachmentOwner in JSON Schema ([581cf33](https://github.com/ken109/linear/commit/581cf3387326f2a014db047941fb90088a019c87))
* **core:** filter the issues an audit fetches ([bbe4dd8](https://github.com/ken109/linear/commit/bbe4dd8fc49182085ea731b64abbba7777777862))
* **core:** find the cycle that contains the day after a meeting ([8f51edb](https://github.com/ken109/linear/commit/8f51edb5ccc3d8e760cabca117d7b858c3b9ef4c))
* **core:** include the description in initiative list JSON ([bfcc157](https://github.com/ken109/linear/commit/bfcc1571fe1c4d53a87fd5e26b9527a2642e00a5))
* **core:** narrow audit to named issues and apply validators to existing ones ([e6a99f5](https://github.com/ken109/linear/commit/e6a99f5c269aeff003fa97fc42120e468bffc546))
* **core:** read and write attachment metadata, and check its kind ([a01d88f](https://github.com/ken109/linear/commit/a01d88f32e1e32547453d7f36a18ec5af0461e43))
* **core:** scan GraphQL documents for their operation kinds ([63aa06b](https://github.com/ken109/linear/commit/63aa06b54b50c3a75d7ef3748c3fbc34a3d12824))
* **core:** select issue description and canceledAt ([9ab46ba](https://github.com/ken109/linear/commit/9ab46ba9ef5b2512ff380051384f70c1a8540a2e))
* **core:** select sortOrder and prioritySortOrder on issues ([9860187](https://github.com/ken109/linear/commit/9860187435d3ad82741036f5a81dd7b850b5286b))
* **core:** serialize query results and describe the boundary types in JSON Schema ([de5bb01](https://github.com/ken109/linear/commit/de5bb01fee613896d0b2c37a9110970ff180bae7))
* **core:** verify Linear webhook signatures ([176d3dc](https://github.com/ken109/linear/commit/176d3dc1aeba5c80d4c83e237eb0384797ecd2aa))
* **guard:** choose how strict the ownership rules are per workspace ([6a2ccc3](https://github.com/ken109/linear/commit/6a2ccc34cf94cb8334dd8ebb9d1d0c03de1ece62))
* per-workspace source title, and retries for reads with a --timeout ([d40d8c5](https://github.com/ken109/linear/commit/d40d8c5262d9eb3d090728499a3de437ca9c2af1))
* **wasm:** add a typed wrapper over the six functions and the scripts that package it ([09d1145](https://github.com/ken109/linear/commit/09d1145cfe0b3ba6bf76e51674b75543d0fde517))
* **wasm:** export build_request and parse_response over a JSON string boundary ([237ca6e](https://github.com/ken109/linear/commit/237ca6ee1252732d9d8cd7f6757a4ae66cea2384))
* **wasm:** export the six functions a Worker needs ([aa57fe4](https://github.com/ken109/linear/commit/aa57fe45c37b767ee6191c34fff9660c0587fa28))
* **wasm:** generate TypeScript types and zod schemas from the Rust types ([e15d861](https://github.com/ken109/linear/commit/e15d86191b5b4cf2703dd872f530adfb6c749d47))


### Bug Fixes

* **audit:** make every fix name a command and flags that exist ([5751f5f](https://github.com/ken109/linear/commit/5751f5fb2ec07ca43e2ccaad5b06138e26325df1))
* **cli:** accept one comma-separated list in issue reorder ([fde37c2](https://github.com/ken109/linear/commit/fde37c280871983f8bef4760b3571307d00653fc))
* **core:** count the age of a status update in calendar days ([bba91f6](https://github.com/ken109/linear/commit/bba91f66c71941769cfd2e5b35f004c6562a45e6))
* **core:** point the overdue issue fix at issue update --due ([01b08b3](https://github.com/ken109/linear/commit/01b08b38acf97135cde95b60591ea9b4a653a418))


### Documentation

* describe --cached reads and the status line ([7d5c4db](https://github.com/ken109/linear/commit/7d5c4dba1210036c3e3959160b87245178ce0f83))
* describe --meta, source_kinds and issue reorder lists ([ee202d9](https://github.com/ken109/linear/commit/ee202d90aea0700a2f93a67f71241bf8072b4233))
* describe issue update's new flags and the audit fix check ([5192b23](https://github.com/ken109/linear/commit/5192b23133dd5d1119001b2c2afec5e6a8990b79))
* describe the cache and the audit, and the data the live audit test needs ([a40942b](https://github.com/ken109/linear/commit/a40942b8fea93fe8920beed5828cdfcffb9cf167))
* describe the pull-based Homebrew tap release flow ([185d2db](https://github.com/ken109/linear/commit/185d2db42bdceade3df49e2beb7c22eb1ba303ad))
* document `linear brief` and close parity gap G1 ([14999e6](https://github.com/ken109/linear/commit/14999e699ffdf31bba8deb2aac371c981e26aa8f))
* document installation and the release process ([99edde8](https://github.com/ken109/linear/commit/99edde8c0b388c6b4cdc1e296b26d41385123e09))
* **parity:** close gaps G3, G5, G6 and G7 ([fab9ec2](https://github.com/ken109/linear/commit/fab9ec28595090fc9094f080247ab525b69bfda6))
* **parity:** list the new issue update and create flags among the commands without an old equivalent ([6ea6706](https://github.com/ken109/linear/commit/6ea670662b331b287a53d1c049a53213981fe59e))
* **parity:** map every command of tools/linear.ts to linear, with differences and gaps ([09d7887](https://github.com/ken109/linear/commit/09d78875e0014a3301afb2eb7c8e45eb77ba6999))
* **readme:** describe the release-please release flow ([f451dd7](https://github.com/ken109/linear/commit/f451dd73e8f97bd2e4a60eb03efcd3dbddfc1b99))
