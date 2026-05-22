FROM python:3.11-slim

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc g++ git && \
    rm -rf /var/lib/apt/lists/*

# Install PyTorch CPU-only first (smaller image, ~200MB vs ~2GB)
RUN pip install --no-cache-dir torch --index-url https://download.pytorch.org/whl/cpu

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Clone Kronos foundation model source
RUN git clone --depth 1 https://github.com/shiyu-coder/Kronos.git /app/kronos_repo

COPY . .

ENV HF_HOME=/app/model_cache

EXPOSE 8000

CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8000", "--reload"]
