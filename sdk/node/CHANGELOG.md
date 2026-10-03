# Changelog

## [2.4.0-beta.8](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.4.0-beta.7...camouflage-tui-v2.4.0-beta.8) (2026-10-03)


### Bug Fixes

* **inline:** no floating cursor bar for a reply with no text yet ([5c32dc1](https://github.com/sinameraji/camouflage/commit/5c32dc193c828ca55e7c7d9e3a03784f2de3f35c))
* **inline:** no floating cursor bar for a reply with no text yet ([776ee38](https://github.com/sinameraji/camouflage/commit/776ee38897f75fefccb4d0a214da6420319dc74b))
* **protocol:** Rust payloads are the source of truth; SDK types can't drift ([8dc4440](https://github.com/sinameraji/camouflage/commit/8dc44408cec7b46b7af941f507d1ae7654b7dd30))
* **protocol:** Rust payloads are the source of truth; SDK types can't drift ([88eb43c](https://github.com/sinameraji/camouflage/commit/88eb43cbbd12a9bd2d197caa6320d2e7e0f8814b))
* **sdk:** release camouflage-tui 2.4.0-beta.8 ([7765b1a](https://github.com/sinameraji/camouflage/commit/7765b1a685553c67569e9edf61e5962ddccf03b4))
* **tui:** crash dumps go to ~/.camouflage, never the working directory ([f678c38](https://github.com/sinameraji/camouflage/commit/f678c38acab681d3a837773b0d0a2a09df199b18))
* **tui:** crash dumps go to ~/.camouflage, never the working directory ([60e9b9d](https://github.com/sinameraji/camouflage/commit/60e9b9db916f2c109da7e881658af42e964fefac))


### Performance Improvements

* **inline:** half the CPU while streaming (ASCII width fast path, 20 fps cap) ([5f6b1d8](https://github.com/sinameraji/camouflage/commit/5f6b1d870d2b214bea671b163dc8e978876b8a68))
* **inline:** half the CPU while streaming (ASCII width fast path, 20 fps cap) ([6ac03e3](https://github.com/sinameraji/camouflage/commit/6ac03e3bbaddac04f42fa89c6d78307fc038ef78))

## [2.4.0-beta.7](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.4.0-beta.6...camouflage-tui-v2.4.0-beta.7) (2026-10-01)


### Features

* **inline:** hand the terminal to a child process (TerminalSuspend/Resume) ([8aa2c99](https://github.com/sinameraji/camouflage/commit/8aa2c99b11e5fbb848983fc25f1243b378f25f2b))
* **inline:** hand the terminal to a child process (TerminalSuspend/Resume) ([7c2a83c](https://github.com/sinameraji/camouflage/commit/7c2a83ceb92fb9311865cdd1b36cb7a70cb5e369))


### Bug Fixes

* **sdk:** release camouflage-tui 2.4.0-beta.7 ([e0cd9de](https://github.com/sinameraji/camouflage/commit/e0cd9de9888171fe50d404da68ca1591c34c291d))

## [2.4.0-beta.6](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.4.0-beta.5...camouflage-tui-v2.4.0-beta.6) (2026-10-01)


### Bug Fixes

* **inline:** info notices are dim hints, not reply-colored text ([5879680](https://github.com/sinameraji/camouflage/commit/5879680ffaab75e6083b9afbeae1179f30a0515e))
* **sdk:** release camouflage-tui 2.4.0-beta.6 ([a638662](https://github.com/sinameraji/camouflage/commit/a6386626330d83e486fb0604bed6d7b5505d5287))

## [2.4.0-beta.5](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.5.0-beta.4...camouflage-tui-v2.4.0-beta.5) (2026-10-01)


### Bug Fixes

* **sdk:** release camouflage-tui 2.4.0-beta.5 ([13b302b](https://github.com/sinameraji/camouflage/commit/13b302b1eb7fc1ef12d5102221a5b6c01620e9fe))
* **sdk:** release camouflage-tui 2.4.0-beta.5 ([5d45aba](https://github.com/sinameraji/camouflage/commit/5d45aba6bc7e714c1ee015d71c8febb444502c21))

## [2.5.0-beta.4](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.4.0-beta.4...camouflage-tui-v2.5.0-beta.4) (2026-10-01)


### Features

* Windows and Intel Mac binaries ([c576e8d](https://github.com/sinameraji/camouflage/commit/c576e8d44a24e24fdbf96ef50c2dc5a1c16c0987))
* Windows and Intel Mac binaries ([a8b4b20](https://github.com/sinameraji/camouflage/commit/a8b4b20356913684976997b13c718be85e300664))

## [2.4.0-beta.4](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.4.0-beta.3...camouflage-tui-v2.4.0-beta.4) (2026-10-01)


### Features

* **inline:** quiet mode: one row per tool, folded reads, faint live output ([3e1a342](https://github.com/sinameraji/camouflage/commit/3e1a3422401b32ce10343953ad11524edee76c07))


### Bug Fixes

* **sdk:** release camouflage-tui 2.4.0-beta.4 ([311ffb6](https://github.com/sinameraji/camouflage/commit/311ffb6efc0089f9c2987cb0a8c90ef4aba307ff))

## [2.4.0-beta.3](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.4.0-beta.2...camouflage-tui-v2.4.0-beta.3) (2026-10-01)


### Bug Fixes

* **inline:** don't print a streamed reply twice when the host sends final text ([3179027](https://github.com/sinameraji/camouflage/commit/3179027030c2458a44c8f8734a643b3c83b200eb))
* **sdk:** release camouflage-tui 2.4.0-beta.3 ([e9007bb](https://github.com/sinameraji/camouflage/commit/e9007bb31dc5706c0bd6c332b9330d9160798a07))

## [2.4.0-beta.2](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.4.0-beta.1...camouflage-tui-v2.4.0-beta.2) (2026-10-01)


### Bug Fixes

* **sdk:** release camouflage-tui 2.4.0-beta.2 ([cc257e7](https://github.com/sinameraji/camouflage/commit/cc257e7f289e0d37571c6d9b7e5a721749b9af64))
* **sdk:** release camouflage-tui 2.4.0-beta.2 (rebuild binary from 9694172f273c) ([ea3515c](https://github.com/sinameraji/camouflage/commit/ea3515c47555cdaaa44dce5412e3ab84f072f7e5))

## [2.4.0-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.3.0-beta.1...camouflage-tui-v2.4.0-beta.1) (2026-10-01)


### Features

* **inline:** multi-line form fields ([050a1d2](https://github.com/sinameraji/camouflage/commit/050a1d2b9500d909ef74ded2050b8400d36ae20c))
* **inline:** multi-line form fields ([1a364e8](https://github.com/sinameraji/camouflage/commit/1a364e87a1ffb1378a7c3e9b9fe181e7f87ab0af))
* **inline:** show the model's reasoning with Ctrl+R ([199bcae](https://github.com/sinameraji/camouflage/commit/199bcae57170c7171727cb50f09e85bbef595562))
* **inline:** show the model's reasoning with Ctrl+R ([a924923](https://github.com/sinameraji/camouflage/commit/a9249233f566785feb498b5335eaed4368b866e6))

## [2.3.0-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.2.2-beta.1...camouflage-tui-v2.3.0-beta.1) (2026-10-01)


### Features

* **inline:** browse folders in @ mentions, and fix two rendering bugs ([b0fde99](https://github.com/sinameraji/camouflage/commit/b0fde99f2f6b2852d3a1590279530c030a402e2e))
* **inline:** browse folders in @ mentions, and fix two rendering bugs ([88169d2](https://github.com/sinameraji/camouflage/commit/88169d2e7a3196b28f62496b178425b2b96bd054))
* **inline:** searchable select lists with sections, columns and toggles ([b1784ba](https://github.com/sinameraji/camouflage/commit/b1784bac8f811855a0d4d0e27112af5d2a0fc623))
* **inline:** searchable select lists with sections, columns and toggles ([ac0578f](https://github.com/sinameraji/camouflage/commit/ac0578f2c1ff4a9ce1adff950f47dcbd51accb55))

## [2.2.2-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.2.1-beta.1...camouflage-tui-v2.2.2-beta.1) (2026-09-30)


### Bug Fixes

* **sdk:** release camouflage-tui 2.2.2-beta.1 (rebuild binary from b5e1ab44466e) ([02f1b38](https://github.com/sinameraji/camouflage/commit/02f1b38b647c738683e10d8d8113550470b6e8f6))
* **tui:** keep the host's event stream clean in inline piped mode ([fb435f9](https://github.com/sinameraji/camouflage/commit/fb435f98f5c7eb00ee78ac54b57607e59c3bdfa7))

## [2.2.1-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.2.0-beta.1...camouflage-tui-v2.2.1-beta.1) (2026-09-30)


### Bug Fixes

* **sdk:** make the default piped mode start, and survive a dead renderer ([4794cef](https://github.com/sinameraji/camouflage/commit/4794cefc85676fccbfe47de29a732970a6185059))

## [2.2.0-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.1.0-beta.1...camouflage-tui-v2.2.0-beta.1) (2026-09-30)


### Features

* **inline:** land the inline renderer stack ([#32](https://github.com/sinameraji/camouflage/issues/32)–[#36](https://github.com/sinameraji/camouflage/issues/36)) ([354ef8f](https://github.com/sinameraji/camouflage/commit/354ef8f51748ba8ecaa76b38cb384460edc9b1cf))
* **sdk:** add inline mode, a permission helper, and typed send ([7e0a4cc](https://github.com/sinameraji/camouflage/commit/7e0a4cc19f1d6d5000796b321b4987ae17a7b1a8))
* **sdk:** add inline mode, a permission helper, and typed send ([7f8b4db](https://github.com/sinameraji/camouflage/commit/7f8b4dbb1178c63815ebc6ef54f43506b585ea09))

## [2.1.0-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v2.0.0-beta.1...camouflage-tui-v2.1.0-beta.1) (2026-06-03)


### Features

* **tui:** rich scrollable task list with progress visualization ([#23](https://github.com/sinameraji/camouflage/issues/23)) ([cd7717f](https://github.com/sinameraji/camouflage/commit/cd7717f41763345a0f444275b17961e3fdb353a9))
* **tui:** rich scrollable task list with progress visualization ([#23](https://github.com/sinameraji/camouflage/issues/23)) ([7f7e1da](https://github.com/sinameraji/camouflage/commit/7f7e1da7f6685879dfd375094703de24938c0528))

## [2.0.0-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v1.1.1-beta.1...camouflage-tui-v2.0.0-beta.1) (2026-05-31)


### ⚠ BREAKING CHANGES

* the Toast component has been removed from the SDK.

### Features

* remove Toast (CC-3) component from the SDK ([da50df1](https://github.com/sinameraji/camouflage/commit/da50df15d6bebdaf0ccfb394ccce7b9ac4133539))

## [1.1.1-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v1.1.0-beta.1...camouflage-tui-v1.1.1-beta.1) (2026-05-31)


### Bug Fixes

* **sdk:** document native binary install + republish to ship Esc-abort fix ([a1cec46](https://github.com/sinameraji/camouflage/commit/a1cec4666db8ba8bbf2a7d599785d66d0edb37d8))
* **sdk:** trigger beta 1.1.1 to ship Esc-abort fix on corrected pipeline ([cf62e63](https://github.com/sinameraji/camouflage/commit/cf62e6336cd633c5718f2ebbf5d525be810aeea0))

## [1.1.0-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v1.0.1-beta.1...camouflage-tui-v1.1.0-beta.1) (2026-05-31)


### Features

* host-supplied header brand; remove client-specific naming ([13a8510](https://github.com/sinameraji/camouflage/commit/13a8510807ac1c85c73fcdda7cb4b880dcc35b3d))
* host-supplied header brand; remove client-specific naming ([2c784be](https://github.com/sinameraji/camouflage/commit/2c784bef92521f102ae440d8e3b443b3ad9e3977))

## [1.0.1-beta.1](https://github.com/sinameraji/camouflage/compare/camouflage-tui-v1.0.0-beta.1...camouflage-tui-v1.0.1-beta.1) (2026-05-29)


### Bug Fixes

* **node sdk:** send() after close() no longer throws — silent no-op ([496035a](https://github.com/sinameraji/camouflage/commit/496035a0e7ce832cc342129bf1c50e367a998d71))
