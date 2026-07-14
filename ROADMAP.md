# Roadmap

This is a contribution map, not a delivery promise. The canonical maturity cockpit is [`docs/product-readiness.md`](docs/product-readiness.md).

## Now

- close the hosted proof gap on #109: real Keycloak, Clever HTTPS/WSS, proxy-log evidence, and a physical-phone smoke;
- freeze the retention defaults and BYOK/provider policy that gate promotion;
- keep the current mono-instance owner/corpus/membership contract explicit while the persistence adapter is designed;
- shape the persistence and multi-instance path for the pilot, including revocation fanout.

## Next

- promote the staging pilot once the #109 hosting gate is proven;
- move from local MVP evidence to a persisted, multi-instance runtime with the same client contract;
- expand the pilot evidence set only after persistence and revocation are observable.

## Later

- broader release automation and scale-out;
- hosted production claims only after the pilot proves the persistence and multi-instance gates.
