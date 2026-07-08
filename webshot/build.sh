#!/bin/bash
set -e

# ── webshot build script — manual APK build without Gradle ──
# Uses: aapt2, kotlinc, d8, zipalign, apksigner (all Termux packages)

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$PROJECT_DIR"

ANDROID_JAR="$PROJECT_DIR/sdk/android-35/android.jar"
KOTLIN_LIB="/data/data/com.termux/files/usr/opt/kotlin/lib"
KOTLIN_STDLIB="$KOTLIN_LIB/kotlin-stdlib.jar"
KOTLIN_JDK8="$KOTLIN_LIB/kotlin-stdlib-jdk8.jar"

BUILD="$PROJECT_DIR/build"
CLASSES="$BUILD/classes"
DEX="$BUILD/dex"
SRC="$PROJECT_DIR/src/com/webshot"

KEYSTORE="$PROJECT_DIR/webshot.keystore"
APK_FINAL="$PROJECT_DIR/webshot.apk"

echo "═══ WebShot Build ═══"

# ── Clean ──
echo "▶ Cleaning…"
rm -rf "$BUILD"
mkdir -p "$CLASSES" "$DEX"

# ── Step 1: Link manifest with aapt2 → base APK ──
echo "▶ Step 1/6: aapt2 link (manifest → base APK)…"
aapt2 link \
    -o "$BUILD/base.apk" \
    --manifest "$PROJECT_DIR/AndroidManifest.xml" \
    -I "$ANDROID_JAR" \
    --min-sdk-version 26 \
    --target-sdk-version 35

# ── Step 2: Compile Kotlin → .class ──
echo "▶ Step 2/6: kotlinc (Kotlin → .class)…"
kotlinc \
    -cp "$ANDROID_JAR:$KOTLIN_STDLIB:$KOTLIN_JDK8" \
    -jvm-target 1.8 \
    -no-stdlib \
    -d "$CLASSES" \
    "$SRC"/*.kt 2>&1 | grep -v "warning:" || true

# ── Step 3: Convert .class + kotlin-stdlib → classes.dex ──
echo "▶ Step 3/6: d8 (.class → DEX)…"
CLASS_FILES=$(find "$CLASSES" -name "*.class" | sort)
d8 \
    --output "$DEX" \
    --lib "$ANDROID_JAR" \
    --min-api 26 \
    $CLASS_FILES \
    "$KOTLIN_STDLIB" \
    "$KOTLIN_JDK8"

# ── Step 4: Add classes.dex to APK ──
echo "▶ Step 4/6: Adding DEX to APK…"
cp "$BUILD/base.apk" "$BUILD/with-dex.apk"
cd "$DEX"
zip -j "$BUILD/with-dex.apk" classes.dex
cd "$PROJECT_DIR"

# ── Step 5: Zipalign ──
echo "▶ Step 5/6: zipalign…"
zipalign -f -p 4 "$BUILD/with-dex.apk" "$BUILD/aligned.apk"

# ── Step 6: Sign ──
echo "▶ Step 6/6: apksigner…"
if [ ! -f "$KEYSTORE" ]; then
    echo "  Generating debug keystore…"
    keytool -genkeypair \
        -keystore "$KEYSTORE" \
        -storepass webshot \
        -keypass webshot \
        -alias webshot \
        -keyalg RSA \
        -keysize 2048 \
        -validity 10000 \
        -dname "CN=WebShot,O=WebShot,C=US"
fi

apksigner sign \
    --ks "$KEYSTORE" \
    --ks-pass pass:webshot \
    --key-pass pass:webshot \
    --out "$APK_FINAL" \
    "$BUILD/aligned.apk"

echo ""
echo "═══ Build complete! ═══"
ls -la "$APK_FINAL"
echo ""
echo "Install:  pm install $APK_FINAL"
echo "Or:       termux-open $APK_FINAL"
