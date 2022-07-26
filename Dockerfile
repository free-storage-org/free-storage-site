FROM rust:slim as build

RUN apt-get update && apt-get install -y perl build-essential

ADD . /src
WORKDIR /src

RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --release --target x86_64-unknown-linux-musl

FROM alpine:3.16.0 as runtime

COPY --from=build /src/target/x86_64-unknown-linux-musl/release/server /server

ENTRYPOINT [ "/server" ]
