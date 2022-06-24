FROM anibali/lua:5.2

# Install Markdown
RUN apk add --update \
    --repository http://dl-1.alpinelinux.org/alpine/edge/testing/ \
    markdown \
    && rm -rf /var/cache/apk/*

# Install ldoc from luarocks
RUN apk add --update gcc lua5.2-dev musl-dev git \
    && export C_INCLUDE_PATH=/usr/include/lua5.2/ \
    && luarocks install ldoc \
    && apk del gcc lua5.2-dev musl-dev git \
    && rm -rf /var/cache/apk/*

VOLUME /app
WORKDIR /app

USER 1000
ENTRYPOINT ["/usr/bin/tini", "--", "ldoc"]