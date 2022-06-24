FROM docker.io/anibali/ldoc:latest
RUN adduser -u 1000 -G root ldoc
USER 1000
ENTRYPOINT ["/usr/bin/tini", "--", "ldoc"]