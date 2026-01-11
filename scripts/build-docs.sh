#!/usr/bin/env bash
# =============================================================================
# build-docs.sh - Build and serve cargo documentation locally
# =============================================================================
#
# Generates rustdoc for all workspace crates and optionally serves it locally.
#
# Usage:
#   ./build-docs.sh [--serve] [--open]
#
# Options:
#   --serve    Start a local HTTP server to view docs (requires `python3`)
#   --open     Open docs in default browser after building
#   --clean    Clean previous docs before building
#

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly DOCS_DIR="${SCRIPT_DIR}/target/doc"
readonly PORT=8000

# Colors
BLUE='\033[0;34m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

serve_docs=false
open_docs=false
clean_first=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --serve) serve_docs=true; shift ;;
        --open) open_docs=true; shift ;;
        --clean) clean_first=true; shift ;;
        --help)
            cat << EOF
Build and serve MorpheusX workspace documentation

Usage: $0 [--serve] [--open] [--clean]

Options:
    --serve     Start HTTP server on localhost:$PORT
    --open      Open docs in browser
    --clean     Clean before building
    --help      Show this help message

Examples:
    # Just build
    $0

    # Build and serve
    $0 --serve

    # Build, serve, and open in browser
    $0 --serve --open
EOF
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

echo -e "${BLUE}=== Building MorpheusX Documentation ===${NC}"

# Clean if requested
if [[ "$clean_first" == "true" ]]; then
    echo "Cleaning previous docs..."
    rm -rf "$DOCS_DIR"
fi

echo "Building docs for all workspace crates..."
echo "(This may take a minute...)"
echo ""

cargo doc \
    --workspace \
    --all-features \
    --no-deps \
    --document-private-items

echo ""
echo -e "${GREEN}✓ Documentation built successfully${NC}"
echo "Location: $DOCS_DIR"
echo ""

# Create root index if it doesn't exist
if [[ ! -f "$DOCS_DIR/index.html" ]]; then
    echo "Creating root index page..."
    cat > "$DOCS_DIR/index.html" << 'EOF'
<!DOCTYPE html>
<html>
<head>
    <title>MorpheusX API Documentation</title>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
            margin: 0;
            padding: 2rem;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
        }
        .container {
            max-width: 900px;
            margin: 0 auto;
            background: white;
            border-radius: 8px;
            padding: 2rem;
            box-shadow: 0 10px 40px rgba(0,0,0,0.2);
        }
        h1 {
            margin-top: 0;
            color: #333;
            border-bottom: 3px solid #667eea;
            padding-bottom: 0.5rem;
        }
        .intro {
            color: #666;
            line-height: 1.6;
            font-size: 1.1rem;
        }
        .crate-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 1.5rem;
            margin-top: 2rem;
        }
        .crate-card {
            border: 1px solid #ddd;
            border-radius: 6px;
            padding: 1.5rem;
            transition: all 0.3s ease;
        }
        .crate-card:hover {
            border-color: #667eea;
            box-shadow: 0 4px 12px rgba(102, 126, 234, 0.2);
            transform: translateY(-2px);
        }
        .crate-card h3 {
            margin: 0 0 0.5rem 0;
            color: #333;
        }
        .crate-card a {
            color: #667eea;
            text-decoration: none;
            font-weight: 600;
        }
        .crate-card a:hover {
            text-decoration: underline;
        }
        .crate-desc {
            color: #666;
            font-size: 0.95rem;
            margin: 0.5rem 0;
        }
        .crate-status {
            display: inline-block;
            font-size: 0.8rem;
            padding: 0.2rem 0.6rem;
            border-radius: 3px;
            margin-top: 0.5rem;
            background: #f0f0f0;
            color: #666;
        }
        .crate-status.stable {
            background: #d4edda;
            color: #155724;
        }
        footer {
            margin-top: 2rem;
            padding-top: 1rem;
            border-top: 1px solid #ddd;
            text-align: center;
            color: #999;
            font-size: 0.9rem;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>🔧 MorpheusX API Documentation</h1>
        
        <div class="intro">
            <p>
                MorpheusX is a UEFI/bare-metal bootloader and exokernel with self-persisting runtime capabilities.
                This documentation covers all crates in the MorpheusX workspace.
            </p>
            <p>
                <strong>Key Features:</strong> Post-EBS networking, ISO9660 support, FAT32 filesystem, 
                UEFI compatibility, and self-updating capabilities.
            </p>
        </div>

        <h2>Workspace Crates</h2>
        <div class="crate-grid">
            <div class="crate-card">
                <h3>📦 morpheus-bootloader</h3>
                <p class="crate-desc">UEFI bootloader and core kernel entry point</p>
                <a href="morpheus_bootloader/index.html">View Docs →</a>
                <span class="crate-status stable">Stable</span>
            </div>

            <div class="crate-card">
                <h3>📚 morpheus-core</h3>
                <p class="crate-desc">Core types, ISO handling, and filesystem abstractions</p>
                <a href="morpheus_core/index.html">View Docs →</a>
                <span class="crate-status stable">Stable</span>
            </div>

            <div class="crate-card">
                <h3>🌐 morpheus-network</h3>
                <p class="crate-desc">Post-EBS network stack with VirtIO drivers and smoltcp integration</p>
                <a href="morpheus_network/index.html">View Docs →</a>
                <span class="crate-status stable">Stable</span>
            </div>

            <div class="crate-card">
                <h3>💾 morpheus-persistent</h3>
                <p class="crate-desc">Self-persistence layer for runtime survival across boots</p>
                <a href="morpheus_persistent/index.html">View Docs →</a>
                <span class="crate-status stable">Stable</span>
            </div>

            <div class="crate-card">
                <h3>🔄 morpheus-updater</h3>
                <p class="crate-desc">In-place update mechanism for bootloader persistence</p>
                <a href="morpheus_updater/index.html">View Docs →</a>
                <span class="crate-status stable">Stable</span>
            </div>

            <div class="crate-card">
                <h3>📀 iso9660-rs</h3>
                <p class="crate-desc">ISO 9660 filesystem parser and writer</p>
                <a href="iso9660_rs/index.html">View Docs →</a>
                <span class="crate-status stable">Stable</span>
            </div>

            <div class="crate-card">
                <h3>🎯 dma-pool</h3>
                <p class="crate-desc">Pre-allocated DMA buffer pool manager for bare-metal I/O</p>
                <a href="dma_pool/index.html">View Docs →</a>
                <span class="crate-status stable">Stable</span>
            </div>
        </div>

        <h2>Documentation Guidelines</h2>
        <ul style="color: #666; line-height: 1.8;">
            <li><strong>Module-level docs:</strong> Each module explains its purpose and usage patterns</li>
            <li><strong>Private items:</strong> Included in docs for implementation reference</li>
            <li><strong>Examples:</strong> See individual crate docs for code examples</li>
            <li><strong>SAFETY comments:</strong> Unsafe code is extensively documented</li>
        </ul>

        <h2>Getting Started</h2>
        <p style="color: #666;">
            Start with <a href="morpheus_bootloader/index.html" style="color: #667eea; font-weight: 600;">morpheus-bootloader</a> 
            for the main entry point, then explore individual crates for specific functionality.
        </p>

        <footer>
            <p>Generated locally • <a href="https://github.com/PopRdi/morpheusx" style="color: #667eea;">View on GitHub</a></p>
        </footer>
    </div>
</body>
</html>
EOF
fi

# Serve if requested
if [[ "$serve_docs" == "true" ]]; then
    echo ""
    echo -e "${YELLOW}Starting documentation server...${NC}"
    echo "URL: ${BLUE}http://localhost:$PORT${NC}"
    echo ""
    echo "Press Ctrl+C to stop the server"
    echo ""
    
    cd "$DOCS_DIR"
    
    # Try Python 3 first, then Python, then fallback
    if command -v python3 &>/dev/null; then
        python3 -m http.server $PORT
    elif command -v python &>/dev/null; then
        python -m http.server $PORT
    else
        echo -e "${YELLOW}⚠ Python not found, cannot start server${NC}"
        echo "Install Python 3 or manually open: file://$DOCS_DIR/index.html"
        exit 1
    fi
fi

# Open in browser if requested
if [[ "$open_docs" == "true" ]]; then
    echo ""
    echo "Opening documentation in browser..."
    
    if command -v xdg-open &>/dev/null; then
        # Linux
        xdg-open "file://$DOCS_DIR/index.html" &
    elif command -v open &>/dev/null; then
        # macOS
        open "file://$DOCS_DIR/index.html"
    else
        echo -e "${YELLOW}⚠ Could not open browser automatically${NC}"
        echo "Open manually: file://$DOCS_DIR/index.html"
    fi
fi
