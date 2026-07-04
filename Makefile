.PHONY: dev-build dev-run dev-shell train-wakeword clean

IMAGE_NAME := waifu-dev
TRAINER_IMAGE := wakeword-trainer

# -- Dev Environment --

dev-build:
	docker build -t $(IMAGE_NAME) -f docker/Dockerfile.dev .

dev-run:
	docker run -it --rm \
		-v /tmp/.X11-unix:/tmp/.X11-unix:ro \
		-e DISPLAY=$$DISPLAY \
		-e XAUTHORITY=$${XAUTHORITY:-$$HOME/.Xauthority} \
		-v $${XAUTHORITY:-$$HOME/.Xauthority}:/tmp/.Xauthority:ro \
		-v $$(pwd):/app:rw \
		-v $$(pwd)/docker/asound.conf:/etc/asound.conf:ro \
		-v /run/user/$$(id -u)/pipewire-0:/tmp/pipewire-0:ro \
		-v waifu-cargo-registry:/usr/local/cargo/registry \
		-v waifu-cargo-git:/usr/local/cargo/git \
		-w /app \
		--device /dev/dri:/dev/dri \
		--device /dev/snd \
		--group-add $$(getent group render | cut -d: -f3) \
		--group-add $$(getent group audio | cut -d: -f3) \
		$(IMAGE_NAME) \
		bash -c "cp /app/.env /app/src-tauri/.env 2>/dev/null; cd /app/src-tauri && bunx tauri dev"

dev-shell:
	docker run -it --rm \
		-v $$(pwd):/app:rw \
		-v waifu-cargo-registry:/usr/local/cargo/registry \
		-v waifu-cargo-git:/usr/local/cargo/git \
		-w /app \
		$(IMAGE_NAME) /bin/bash

# -- Wake Word Training --

-include .env

train-wakeword:
	@if [ -z "$(WORD)" ]; then \
		echo "Usage: make train-wakeword WORD=kassandra"; \
		exit 1; \
	fi
	@mkdir -p wakeword-configs wakeword-output
	docker build --build-arg HF_TOKEN=$(HF_TOKEN) -t $(TRAINER_IMAGE) -f docker/wakeword-trainer/Dockerfile docker/wakeword-trainer/
	docker run --rm \
		-e HF_TOKEN=$(HF_TOKEN) \
		-v $$(pwd)/wakeword-configs:/workspace/configs \
		-v $$(pwd)/wakeword-output:/workspace/output \
		$(TRAINER_IMAGE) \
		livekit-wakeword run configs/$(WORD).yaml
	@echo "Trained model: wakeword-output/$(WORD)/$(WORD).onnx"
	@echo "Copy to: src-tauri/models/$(WORD).onnx"

# -- Cleanup --

clean:
	rm -rf wakeword-output/
	cargo clean --manifest-path src-tauri/Cargo.toml
