# Changelog

## [0.11.0](https://github.com/protoc-contrib/aip-rs/compare/v0.10.1...v0.11.0) (2026-10-19)


### Features

* build a FieldPath from a field's name, checked against the message ([#39](https://github.com/protoc-contrib/aip-rs/issues/39)) ([964b9f7](https://github.com/protoc-contrib/aip-rs/commit/964b9f70d7615bc46c2c4b2ca0c92bd1a5b7e6e3))

## [0.10.1](https://github.com/protoc-contrib/aip-rs/compare/v0.10.0...v0.10.1) (2026-10-15)


### Bug Fixes

* box a FieldError's payload, so a Result of it stays small ([#35](https://github.com/protoc-contrib/aip-rs/issues/35)) ([6eac286](https://github.com/protoc-contrib/aip-rs/commit/6eac286badd9c1e7d7ac8ccb2b8e693b65824af6))

## [0.10.0](https://github.com/protoc-contrib/aip-rs/compare/v0.9.0...v0.10.0) (2026-10-15)


### ⚠ BREAKING CHANGES

* Resource drops Name and parse_name, which nothing called ([#34](https://github.com/protoc-contrib/aip-rs/issues/34))

### Features

* FieldError, a failure and the field it was of ([#32](https://github.com/protoc-contrib/aip-rs/issues/32)) ([ba5f6a1](https://github.com/protoc-contrib/aip-rs/commit/ba5f6a1d9085b8641e6942efa33e5ac886e62402))


### Code Refactoring

* Resource drops Name and parse_name, which nothing called ([#34](https://github.com/protoc-contrib/aip-rs/issues/34)) ([038c952](https://github.com/protoc-contrib/aip-rs/commit/038c952bf8a4438c8b8b92cb074b448060918f5f))

## [0.9.0](https://github.com/protoc-contrib/aip-rs/compare/v0.8.0...v0.9.0) (2026-10-15)


### ⚠ BREAKING CHANGES

* Procedure::unary and Procedure::streaming are replaced by Procedure::new and Procedure::clearing; the validation error's message is fixed.

### Features

* apply field behavior to every message; answer a validation error with a fixed message ([#29](https://github.com/protoc-contrib/aip-rs/issues/29)) ([19d32fd](https://github.com/protoc-contrib/aip-rs/commit/19d32fdcb3e6bc8c92c4028a3a5fa5d33e76d167))

## [0.8.0](https://github.com/protoc-contrib/aip-rs/compare/v0.7.0...v0.8.0) (2026-10-15)


### Features

* carry a validation error's violations as a google.rpc.BadRequest too ([#27](https://github.com/protoc-contrib/aip-rs/issues/27)) ([0536193](https://github.com/protoc-contrib/aip-rs/commit/0536193e9419292300381846b10b1034f19a78fb))

## [0.7.0](https://github.com/protoc-contrib/aip-rs/compare/v0.6.0...v0.7.0) (2026-10-15)


### Features

* FieldBehaviorSet Display, a ParseError for a bad id field, and interceptor tests through a server ([#25](https://github.com/protoc-contrib/aip-rs/issues/25)) ([e10a868](https://github.com/protoc-contrib/aip-rs/commit/e10a868f1c602c3dff95abee21a9e2bab8f62614))

## [0.6.0](https://github.com/protoc-contrib/aip-rs/compare/v0.5.0...v0.6.0) (2026-10-14)


### ⚠ BREAKING CHANGES

* Resource requires Default, and DynResource has a new required method. Generated resources are unaffected.

### Features

* ignore OUTPUT_ONLY paths in an update mask, with clear_field_mask ([#22](https://github.com/protoc-contrib/aip-rs/issues/22)) ([e188955](https://github.com/protoc-contrib/aip-rs/commit/e1889555479f7ab2ee37fe7741ab017714ac77e7))

## [0.5.0](https://github.com/protoc-contrib/aip-rs/compare/v0.4.0...v0.5.0) (2026-10-13)


### ⚠ BREAKING CHANGES

* FieldBehavior's MUTABLE_PATHS, implied_update_mask, imply_resource_name, imply_update_mask, imply_request, validate_required, validate_required_fields, validate_immutable, clear_output_only and clear_input_only are removed, as are RequiredError and ImmutableError. Use clear_fields(kind), validate_fields(kind, scope) and FieldBehaviorError, and Resource::implied_field_mask for the implied mask.

### Features

* FieldBehavior clears and validates by kind, and a Resource trait ([#19](https://github.com/protoc-contrib/aip-rs/issues/19)) ([2c5e7d6](https://github.com/protoc-contrib/aip-rs/commit/2c5e7d664dbda0cb5dc02cca741fed91f936f249))

## [0.4.0](https://github.com/protoc-contrib/aip-rs/compare/v0.3.0...v0.4.0) (2026-10-12)


### ⚠ BREAKING CHANGES

* missing_required_request_paths, missing_required_paths and changed_immutable_paths are removed in favour of validate_required, validate_required_fields and validate_immutable, which return Result<(), RequiredError> / Result<(), ImmutableError>.

### Features

* validate_* and imply_* field behavior, with RequiredError and ImmutableError ([#17](https://github.com/protoc-contrib/aip-rs/issues/17)) ([c0798c3](https://github.com/protoc-contrib/aip-rs/commit/c0798c35a9e750cc770bb64e89653f997f46c23b))

## [0.3.0](https://github.com/protoc-contrib/aip-rs/compare/v0.2.0...v0.3.0) (2026-10-12)


### Features

* add request-level FieldBehavior items and FieldPath ([#15](https://github.com/protoc-contrib/aip-rs/issues/15)) ([541dc2e](https://github.com/protoc-contrib/aip-rs/commit/541dc2e72f041bf48964b804c4ca0265a5abd5d5))

## [0.2.0](https://github.com/protoc-contrib/aip-rs/compare/v0.1.0...v0.2.0) (2026-10-12)


### Features

* add the FieldBehavior trait generated code implements ([#13](https://github.com/protoc-contrib/aip-rs/issues/13)) ([6dc7a18](https://github.com/protoc-contrib/aip-rs/commit/6dc7a18d0414305936ce1eb7747fe64e37827b73))

## 0.1.0 (2026-09-25)


### Features

* hold resource patterns to AIP-122 spelling ([#8](https://github.com/protoc-contrib/aip-rs/issues/8)) ([a5fef9c](https://github.com/protoc-contrib/aip-rs/commit/a5fef9c544ba6410c2ee3f95ce93d10eb795d8e3))
* name the resource-name error ParseError ([#5](https://github.com/protoc-contrib/aip-rs/issues/5)) ([a2c2dc9](https://github.com/protoc-contrib/aip-rs/commit/a2c2dc99d2ee3379ef2e86cb13da9cce447283fe))
