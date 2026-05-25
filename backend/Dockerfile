FROM nvidia/cuda:12.8.1-runtime-ubuntu22.04

ENV DEBIAN_FRONTEND=noninteractive

WORKDIR /app

# Install Python 3.11 from deadsnakes PPA + build tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    software-properties-common && \
    add-apt-repository ppa:deadsnakes/ppa && \
    apt-get update && apt-get install -y --no-install-recommends \
    python3.11 python3.11-venv python3.11-dev python3.11-distutils \
    gcc g++ git curl && \
    ln -sf /usr/bin/python3.11 /usr/bin/python && \
    ln -sf /usr/bin/python3.11 /usr/bin/python3 && \
    curl -sS https://bootstrap.pypa.io/get-pip.py | python3.11 && \
    rm -rf /var/lib/apt/lists/*

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Install PyTorch with CUDA 13.2 support AFTER requirements (required for RTX 50-series / Blackwell sm_120)
# Let pip pull nvidia-cublas-cu132 etc. so torch has its CUDA runtime libs
RUN pip install --no-cache-dir --pre torch --index-url https://download.pytorch.org/whl/nightly/cu132 --force-reinstall

# Clone Kronos foundation model source
RUN git clone --depth 1 https://github.com/shiyu-coder/Kronos.git /app/kronos_repo

COPY . .

ENV HF_HOME=/app/model_cache

EXPOSE 8000

CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8000", "--reload"]
