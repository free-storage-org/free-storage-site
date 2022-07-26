FROM rust:alpine as build

RUN apk add --no-cache --update g++ make perl openssl-dev

ADD . /src
WORKDIR /src

RUN cargo build --release

FROM alpine:3.16 as runtime

COPY --from=build /src/target/release/free-storage /free-storage

ENTRYPOINT [ "/free-storage" ]
