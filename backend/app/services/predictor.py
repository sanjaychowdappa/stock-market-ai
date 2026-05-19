import numpy as np
import pandas as pd
from sklearn.ensemble import GradientBoostingRegressor, RandomForestRegressor
from sklearn.preprocessing import StandardScaler
from xgboost import XGBRegressor


def prepare_features(df: pd.DataFrame, lookback: int = 10) -> tuple:
    feature_cols = [
        "Open", "High", "Low", "Close", "Volume",
        "SMA_20", "SMA_50", "EMA_12", "EMA_26",
        "RSI", "MACD", "MACD_Signal", "MACD_Hist",
        "BB_Upper", "BB_Lower", "ATR", "Stoch_K", "Stoch_D", "ADX", "OBV",
    ]
    available = [c for c in feature_cols if c in df.columns]
    data = df[available].values

    X, y_close, y_high, y_low = [], [], [], []
    for i in range(lookback, len(data) - 1):
        X.append(data[i - lookback:i].flatten())
        y_close.append(df["Close"].iloc[i + 1])
        y_high.append(df["High"].iloc[i + 1])
        y_low.append(df["Low"].iloc[i + 1])

    return np.array(X), np.array(y_close), np.array(y_high), np.array(y_low)


def predict_next_candle(df: pd.DataFrame) -> dict:
    if len(df) < 30:
        return {"error": "Not enough data for prediction"}

    lookback = 10
    X, y_close, y_high, y_low = prepare_features(df, lookback)

    if len(X) < 20:
        return {"error": "Not enough samples"}

    scaler = StandardScaler()
    X_scaled = scaler.fit_transform(X)

    split = int(len(X_scaled) * 0.8)
    X_train, X_test = X_scaled[:split], X_scaled[split:]

    models = {
        "xgboost": XGBRegressor(n_estimators=100, max_depth=5, learning_rate=0.1, verbosity=0),
        "gradient_boost": GradientBoostingRegressor(n_estimators=100, max_depth=5),
        "random_forest": RandomForestRegressor(n_estimators=100, max_depth=10),
    }

    predictions = {}
    for target_name, y_target in [("close", y_close), ("high", y_high), ("low", y_low)]:
        y_train = y_target[:split]
        model_preds = []

        for name, model in models.items():
            model.fit(X_train, y_train)
            last_features = X_scaled[-1:].copy()
            pred = model.predict(last_features)[0]
            model_preds.append(pred)

        predictions[target_name] = float(np.mean(model_preds))

    last_close = float(df["Close"].iloc[-1])
    pred_close = predictions["close"]
    pred_open = last_close + (pred_close - last_close) * 0.2

    confidence = _calculate_confidence(df, predictions)

    return {
        "predicted_open": round(pred_open, 2),
        "predicted_high": round(predictions["high"], 2),
        "predicted_low": round(predictions["low"], 2),
        "predicted_close": round(pred_close, 2),
        "current_close": round(last_close, 2),
        "change_percent": round((pred_close - last_close) / last_close * 100, 2),
        "direction": "bullish" if pred_close > last_close else "bearish",
        "confidence": round(confidence, 2),
        "models_used": list(models.keys()),
    }


def _calculate_confidence(df: pd.DataFrame, preds: dict) -> float:
    recent_volatility = df["Close"].iloc[-20:].std() / df["Close"].iloc[-20:].mean()
    volatility_factor = max(0.3, 1 - recent_volatility * 10)

    rsi = df["RSI"].iloc[-1] if "RSI" in df.columns else 50
    rsi_factor = 1 - abs(rsi - 50) / 100

    adx = df["ADX"].iloc[-1] if "ADX" in df.columns else 25
    trend_factor = min(adx / 50, 1.0)

    return min(0.95, (volatility_factor * 0.4 + rsi_factor * 0.3 + trend_factor * 0.3))


def predict_multi_step(df: pd.DataFrame, steps: int = 5) -> list[dict]:
    results = []
    working_df = df.copy()

    for step in range(steps):
        pred = predict_next_candle(working_df)
        if "error" in pred:
            break

        pred["step"] = step + 1
        results.append(pred)

        new_row = working_df.iloc[-1:].copy()
        new_row["Open"] = pred["predicted_open"]
        new_row["High"] = pred["predicted_high"]
        new_row["Low"] = pred["predicted_low"]
        new_row["Close"] = pred["predicted_close"]
        working_df = pd.concat([working_df, new_row], ignore_index=True)

    return results
