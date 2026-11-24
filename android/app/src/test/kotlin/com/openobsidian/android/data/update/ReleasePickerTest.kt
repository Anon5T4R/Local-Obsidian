package com.openobsidian.android.data.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class ReleasePickerTest {

    private val hex = "a".repeat(64)

    private fun asset(name: String, digest: String? = null, url: String? = null) = """
        {"name":"$name","size":12345,
         "browser_download_url":"${url ?: "https://github.com/Anon5T4R/Local-Obsidian/releases/download/x/$name"}",
         "digest":${if (digest == null) "null" else "\"$digest\""}}
    """.trimIndent()

    private fun release(tag: String, assets: List<String>, draft: Boolean = false, prerelease: Boolean = false) = """
        {"tag_name":"$tag","draft":$draft,"prerelease":$prerelease,"assets":[${assets.joinToString(",")}]}
    """.trimIndent()

    private val ok = release("v1.10.0", listOf(
        asset("SHA256SUMS"),
        asset("localobsidian-1.10.0-windows-x86_64.msi"),
        asset("LocalNotebook-Android-v1.10.0.apk", digest = "sha256:$hex"),
    ))

    @Test
    fun pega_o_apk_da_release_mais_recente() {
        val r = ReleasePicker.latest(ok)!!
        assertEquals("1.10.0", r.version)
        assertEquals("v1.10.0", r.tag)
        assertEquals("LocalNotebook-Android-v1.10.0.apk", r.apkName)
        assertEquals("https://github.com/Anon5T4R/Local-Obsidian/releases/download/x/LocalNotebook-Android-v1.10.0.apk", r.apkUrl)
        assertEquals(12345L, r.apkSize)
        assertEquals(hex, r.sha256)
    }

    @Test
    fun digest_em_maiusculas_e_normalizado_e_ausente_vira_null() {
        val up = release("v1.10.0", listOf(asset("a.apk", digest = "sha256:" + hex.uppercase())))
        assertEquals(hex, ReleasePicker.latest(up)!!.sha256)
        assertNull(ReleasePicker.latest(release("v1.10.0", listOf(asset("a.apk"))))!!.sha256)
    }

    @Test
    fun compara_a_tag_com_a_versao_instalada() {
        assertEquals("1.10.0", ReleasePicker.newerThan(ok, "1.9.3")?.version)
        assertNull(ReleasePicker.newerThan(ok, "1.10.0"))
        assertNull(ReleasePicker.newerThan(ok, "1.10.1"))
        assertEquals("1.10.0", ReleasePicker.newerThan(ok, "1.10.0-rc.1")?.version)
        // 1.10.0 vence 1.9.3 na comparacao numerica, nao na de texto.
        assertEquals("1.10.0", ReleasePicker.newerThan(ok, "1.9.9")?.version)
    }

    @Test
    fun release_sem_apk_rascunho_pre_release_ou_tag_estranha_nao_serve() {
        assertNull(ReleasePicker.latest(release("v1.10.0", listOf(asset("a.msi"), asset("SHA256SUMS")))))
        assertNull(ReleasePicker.latest(release("v1.10.0", listOf(asset("a.apk")), draft = true)))
        assertNull(ReleasePicker.latest(release("v1.10.0", listOf(asset("a.apk")), prerelease = true)))
        assertNull(ReleasePicker.latest(release("v1.11.0-rc.1", listOf(asset("a.apk")))))
        assertNull(ReleasePicker.latest(release("android-v3.6.0", listOf(asset("a.apk")))))
        assertNull(ReleasePicker.latest(release("vlixo", listOf(asset("a.apk")))))
    }

    @Test
    fun so_baixa_do_repositorio_do_app() {
        listOf(
            "http://github.com/evil.apk",
            "https://example.com/evil.apk",
            "https://github.com/Anon5T4R/Local-Keys/releases/download/x/evil.apk",
        ).forEach { url ->
            assertNull(url, ReleasePicker.latest(release("v1.10.0", listOf(asset("evil.apk", url = url)))))
        }
        // Um asset suspeito nao esconde o bom que vem depois.
        val mixed = release("v1.10.0", listOf(asset("evil.apk", url = "https://example.com/evil.apk"), asset("bom.apk")))
        assertEquals("bom.apk", ReleasePicker.latest(mixed)!!.apkName)
    }

    @Test
    fun json_invalido_ou_resposta_de_erro_vira_falha_de_parse() {
        listOf("", "[]", "{\"message\":\"API rate limit exceeded\"}", "<html>").forEach { body ->
            try {
                ReleasePicker.latest(body)
                fail("esperava UpdateException para: $body")
            } catch (e: UpdateException) {
                assertEquals(UpdateFailure.PARSE, e.failure)
            }
        }
    }

    @Test
    fun release_guardada_vai_e_volta_e_a_corrompida_e_ignorada() {
        val r = ReleasePicker.latest(ok)!!
        assertEquals(r, AndroidRelease.fromJson(r.toJson()))
        val semDigest = r.copy(sha256 = null)
        assertEquals(semDigest, AndroidRelease.fromJson(semDigest.toJson()))
        assertNull(AndroidRelease.fromJson(null))
        assertNull(AndroidRelease.fromJson("{"))
        assertNull(AndroidRelease.fromJson(r.copy(apkUrl = "https://example.com/x.apk").toJson()))
        assertTrue(ReleasePicker.isAllowedDownload(r.apkUrl))
    }
}
