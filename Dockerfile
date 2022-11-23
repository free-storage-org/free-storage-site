FROM rust:1.65.0-alpine3.16 as build

RUN apk add --no-cache --update g++ make perl openssl-dev

ADD . /src
WORKDIR /src

RUN cargo build --release

FROM alpine:3.16 as runtime

COPY --from=build /src/target/release/site /site

ENTRYPOINT [ "/site" ]
