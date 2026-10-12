# Changelog

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
