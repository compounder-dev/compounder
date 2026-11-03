# The whole app on one machine: the web app and its API, and the desk that runs the funds.

FROM rust:1.98-bookworm AS build
# The web build needs Node and builds the engine for the browser.
COPY --from=node:22-bookworm /usr/local/bin/node /usr/local/bin/node
COPY --from=node:22-bookworm /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s /usr/local/lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
 && rustup target add wasm32-unknown-unknown
WORKDIR /app
COPY . .
RUN cargo build --release -p compounder-server -p compounder-desk
RUN cd web && npm ci && npm run build

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/compounder-server /app/target/release/compounder-desk /usr/local/bin/
# The server finds the built web app beside its own source folder.
COPY --from=build /app/web/dist /app/web/dist
RUN mkdir -p /app/server
COPY start.sh /usr/local/bin/start.sh
CMD ["/usr/local/bin/start.sh"]
