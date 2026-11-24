package com.openobsidian.android.data.update

import org.json.JSONException
import org.json.JSONObject

/** Uma release do Local Notebook Android publicada no GitHub, com o APK que a atualização baixa. */
data class AndroidRelease(
    val version: String,
    val tag: String,
    val apkName: String,
    val apkUrl: String,
    val apkSize: Long,
    /** SHA-256 em hex minúsculo do campo `digest` do asset; null quando o GitHub não o publicou. */
    val sha256: String?,
) {
    /** Forma guardada nas preferências (a última release achada sobrevive ao reinício). */
    fun toJson(): String = JSONObject()
        .put("version", version)
        .put("tag", tag)
        .put("apkName", apkName)
        .put("apkUrl", apkUrl)
        .put("apkSize", apkSize)
        .put("sha256", sha256 ?: JSONObject.NULL)
        .toString()

    companion object {
        fun fromJson(raw: String?): AndroidRelease? {
            if (raw.isNullOrEmpty()) return null
            return try {
                val o = JSONObject(raw)
                AndroidRelease(
                    version = o.getString("version"),
                    tag = o.getString("tag"),
                    apkName = o.getString("apkName"),
                    apkUrl = o.getString("apkUrl"),
                    apkSize = o.getLong("apkSize"),
                    sha256 = if (o.isNull("sha256")) null else Sha256.normalize(o.getString("sha256")),
                ).takeIf { ReleasePicker.isAllowedDownload(it.apkUrl) && AppVersion.parse(it.version) != null }
            } catch (_: JSONException) {
                null
            }
        }
    }
}

/**
 * Lê a release mais recente do repositório (`GET /repos/{owner}/{repo}/releases/latest`), que é a do desktop e do
 * Android juntos: tag `vX.Y.Z`, com o `.apk` como mais um asset. Vale se não for rascunho nem pré-release, se a tag for
 * semver e se trouxer um `.apk`; a versão é a da tag (igual ao `versionName`, `cargo xtask version` mantém os dois).
 * Pura (org.json), testada na JVM.
 */
object ReleasePicker {
    /** Só o que este repositório publica (o Local Notebook, repo do desktop com o Android em `android/`): o APK vem de `releases/download/` dele e de mais lugar nenhum. */
    const val DOWNLOAD_PREFIX = "https://github.com/Anon5T4R/Local-Obsidian/releases/download/"

    /** A release mais recente com APK, ou null se ela não serve (sem APK, rascunho, pré-release, tag que não é semver). Lança [UpdateException] (PARSE). */
    fun latest(json: String): AndroidRelease? {
        val release = try {
            JSONObject(json)
        } catch (_: JSONException) {
            throw UpdateException(UpdateFailure.PARSE)
        }
        // A API responde {"message": ...} quando falha (limite de taxa, 404): sem tag_name não é uma release.
        if (!release.has("tag_name")) throw UpdateException(UpdateFailure.PARSE)
        return parseRelease(release)
    }

    /** A release mais recente se ela for mais nova que [currentVersion]; null quando o app está em dia. */
    fun newerThan(json: String, currentVersion: String): AndroidRelease? =
        latest(json)?.takeIf { AppVersion.isNewer(it.version, currentVersion) }

    /** O APK só vem das releases deste repositório, por https (o GitHub redireciona para o próprio CDN, também por https). */
    fun isAllowedDownload(url: String): Boolean = url.startsWith(DOWNLOAD_PREFIX)

    private fun parseRelease(o: JSONObject): AndroidRelease? {
        if (o.optBoolean("draft", false) || o.optBoolean("prerelease", false)) return null
        val tag = o.optString("tag_name", "")
        // Só `vX.Y.Z`: o prefixo é parte do contrato (as tags antigas `android-v…` não valem).
        if (!tag.startsWith("v")) return null
        val version = tag.removePrefix("v")
        val parsed = AppVersion.parse(version) ?: return null
        if (parsed.pre.isNotEmpty()) return null
        val assets = o.optJSONArray("assets") ?: return null
        for (i in 0 until assets.length()) {
            val asset = assets.optJSONObject(i) ?: continue
            val name = asset.optString("name", "")
            val url = asset.optString("browser_download_url", "")
            if (!name.endsWith(".apk", ignoreCase = true) || !isAllowedDownload(url)) continue
            val digest = if (asset.isNull("digest")) null else asset.optString("digest", "")
            return AndroidRelease(
                version = version,
                tag = tag,
                apkName = name,
                apkUrl = url,
                apkSize = asset.optLong("size", -1L),
                sha256 = Sha256.fromGithubDigest(digest),
            )
        }
        return null
    }
}
