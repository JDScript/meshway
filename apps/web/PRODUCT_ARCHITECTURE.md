# Console product architecture

The console was rebuilt on 2026-10-01 around network-scoped URLs and a
Tailscale-style top navigation. The design source of truth is
[`docs/console-design.md`](./docs/console-design.md): information
architecture, task flows, Tailscale mapping, visual language, component
rules and wireframes for pages that still lack a backend.

## Code map

| Concern | Location |
| --- | --- |
| Network-scoped routes and old URL mapping | `lib/routes.ts`, `components/shell/legacy-redirect.tsx` |
| Server scope adapter (`/<network_id>` → controller) | `lib/network-scope.ts`, `lib/controller.ts` |
| Action scope and login checks | `lib/network-context.ts`, `lib/session.ts`, `lib/*-actions.ts` |
| Shell, navigation, network switcher, ⌘K | `components/shell/*` |
| Per-network draft stores | `components/network/network-workspace.tsx` |
| Design tokens | `app/globals.css` |
| Shared surfaces and states | `components/common/*` |
| App same-origin endpoints | `app/api/v1/**`, `lib/node-proxy.ts` |

Pages live in `app/(console)/[network]/…`; `(policies)` and `(diagnostics)`
route groups only add their section tabs. Every page calls `requireNetwork`
and every network action takes the rendered network ID as its first argument.
