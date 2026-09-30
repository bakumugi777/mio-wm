# Window Rules and Properties

[日本語版](window-properties-jp.md)

Mio uses one Property system for Window behavior and appearance. The currently available Properties are `opacity`, `floating`, and `blur`.

## Effective-value precedence

Each Property independently resolves its effective value in this order:

```text
Default < Matched Config Rules < Runtime Override
```

If a layer does not define that Property, resolution falls back to the layer below it. Clearing a runtime override restores the matching Config Rule value, or the Default if no rule defines it. The default opacity is configured with `appearance.opacity`. It applies only to Windows, not as alpha for the entire scene including the background and layer-shell surfaces.

Runtime overrides are per Window ID. They do not affect other Windows with the same app ID and do not rewrite the handwritten KDL file. An override expires when its Window closes.

## Rule matching

A `window-rule` uses exact `app-id` and `title` matching and requires at least one of them. Specifying both creates an AND condition. Regular expressions and globs are not currently supported.

```kdl
window-rule {
    match app-id="foot"
    opacity 0.9
}

window-rule {
    match app-id="foot" title="main"
    blur true
    opacity 1.0
}
```

When several rules match, their Properties are composed in file order. A later rule overrides only a Property also set by an earlier matching rule. In the example above, a `foot` Window titled `main` receives `opacity=1.0` and `blur=true` in the effective Config layer.

Mio recalculates the complete Config Rule layer for existing Windows when a client changes its app ID or title, and after a successful configuration reload. Values from rules that no longer match do not remain. Runtime overrides are retained and continue to take precedence for their respective Properties.

## Floating

`floating=true` relaxes Grid collision constraints within the same World. It does not move a Window to another workspace or coordinate system. A runtime `floating=false` overrides a Config Rule's `true`; clearing it restores the rule value.

If a Config Rule makes a new Window floating before its first presentation, Mio initially places it over the Window that was focused immediately before launch. This applies only to initial placement: configuration reloads and runtime floating changes do not move existing Windows.

## Configuration reload

`reload-config` replaces the configuration only after the entire new configuration validates successfully. An invalid configuration preserves the last valid configuration and Properties. A successful reload reapplies rules to existing Windows but does not rerun startup commands.
