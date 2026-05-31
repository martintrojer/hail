FROM docker.io/library/alpine:3.20

RUN apk add --no-cache bash curl jq

COPY init.sh /usr/local/bin/stalwart-init
RUN chmod 0755 /usr/local/bin/stalwart-init

ENTRYPOINT ["/usr/local/bin/stalwart-init"]
