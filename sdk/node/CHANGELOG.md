# Changelog

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
