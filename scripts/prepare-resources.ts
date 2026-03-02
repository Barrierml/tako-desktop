/**
 * Build Hub + Web and copy artifacts into src-tauri/resources/
 * so Tauri can bundle them into the application.
 *
 * Two modes:
 * - Monorepo mode (local dev): builds hub/cli/web from sibling packages
 * - Standalone mode (CI / independent repo): downloads pre-built bundles from npm
 */
import { $ } from 'bun'
import { join, resolve } from 'path'
import { mkdir, cp, access, rm } from 'fs/promises'
import { existsSync } from 'fs'

const DESKTOP_DIR = resolve(import.meta.dir, '..')
const ROOT = resolve(DESKTOP_DIR, '../..')
const RESOURCES = join(DESKTOP_DIR, 'src-tauri/resources')

const BUNDLES_PKG = 'tako-desktop-bundles'

function isMonorepo(): boolean {
    return existsSync(join(ROOT, 'packages/hub/package.json'))
}

async function prepareMonorepo() {
    const HUB_DIR = join(ROOT, 'packages/hub')
    const CLI_DIR = join(ROOT, 'packages/cli')
    const WEB_DIR = join(HUB_DIR, 'web')

    console.log('[prepare] Monorepo mode detected')

    console.log('[prepare] Building Hub...')
    await $`bun run build`.cwd(HUB_DIR)

    console.log('[prepare] Building Web UI...')
    await $`bun run build`.cwd(WEB_DIR)

    // Copy artifacts
    console.log('[prepare] Copying hub-bundle.js...')
    const hubBundle = join(HUB_DIR, 'dist/index.js')
    await access(hubBundle) // throws if missing
    await cp(hubBundle, join(RESOURCES, 'hub-bundle.js'))

    console.log('[prepare] Copying web-dist/...')
    const webDist = join(WEB_DIR, 'dist')
    await access(webDist) // throws if missing
    await mkdir(join(RESOURCES, 'web-dist'), { recursive: true })
    await cp(webDist, join(RESOURCES, 'web-dist'), { recursive: true })

    // Build CLI so autoRunner can find it adjacent to hub-bundle.js
    console.log('[prepare] Building CLI...')
    await $`bun run build`.cwd(CLI_DIR)

    console.log('[prepare] Copying cli-bundle.js...')
    const cliBundle = join(CLI_DIR, 'dist/index.js')
    await access(cliBundle) // throws if missing
    await cp(cliBundle, join(RESOURCES, 'cli-bundle.js'))

    // Copy sidecar files that Hub loads via import.meta.dir at runtime
    console.log('[prepare] Copying catalog.json...')
    const catalog = join(HUB_DIR, 'src/marketplace/catalog.json')
    await cp(catalog, join(RESOURCES, 'catalog.json'))
}

async function prepareStandalone() {
    console.log('[prepare] Standalone mode — downloading bundles from npm...')

    // Install the bundles package temporarily
    await $`bun add ${BUNDLES_PKG} --no-save`.cwd(DESKTOP_DIR)

    const bundlesDir = join(DESKTOP_DIR, 'node_modules', BUNDLES_PKG)
    await access(bundlesDir) // throws if missing

    // Copy each artifact
    const files = ['hub-bundle.js', 'cli-bundle.js', 'catalog.json']
    for (const file of files) {
        const src = join(bundlesDir, file)
        console.log(`[prepare] Copying ${file}...`)
        await cp(src, join(RESOURCES, file))
    }

    // Copy web-dist directory
    console.log('[prepare] Copying web-dist/...')
    const webDistSrc = join(bundlesDir, 'web-dist')
    await mkdir(join(RESOURCES, 'web-dist'), { recursive: true })
    await cp(webDistSrc, join(RESOURCES, 'web-dist'), { recursive: true })

    // Clean up
    console.log('[prepare] Cleaning up temporary package...')
    await rm(join(DESKTOP_DIR, 'node_modules', BUNDLES_PKG), { recursive: true, force: true })
}

async function main() {
    if (isMonorepo()) {
        await prepareMonorepo()
    } else {
        await prepareStandalone()
    }

    console.log('[prepare] Done.')
}

main().catch((err) => {
    console.error('[prepare] Failed:', err)
    process.exit(1)
})
