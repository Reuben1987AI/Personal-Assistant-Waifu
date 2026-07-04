FROM python:3.11-slim

ARG HF_TOKEN

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

RUN pip install --no-cache-dir "livekit-wakeword[train,eval,export]"

RUN pip install --no-cache-dir "huggingface_hub[hf_transfer]"

ENV HF_HUB_ENABLE_HF_TRANSFER=1

RUN HF_TOKEN=$HF_TOKEN livekit-wakeword setup

RUN mkdir -p /workspace && mv /data /workspace/data

RUN apt-get update && apt-get install -y --no-install-recommends espeak-ng \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
