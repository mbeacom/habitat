FROM ubuntu:18.04

RUN apt-get update

RUN apt-get install -y \
            libarchive-dev \
            libczmq-dev \
            libsodium-dev \
            libssl-dev

RUN apt-get install -y \
            jq \
            curl

RUN useradd --user-group hab

RUN curl https://raw.githubusercontent.com/habitat-sh/habitat/master/components/hab/install.sh | bash
