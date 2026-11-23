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
         "browser_download_url":"${url ?: "https://github.com/Anon5T4R/OpenObsidianAndroid/releases/download/x/$name"}",
         "digest":${if (digest == null) "null" else "\"$digest\""}}
    """.trimIndent()

    private fun release(tag: String, assets: List<String>, draft: Boolean = false, prerelease: Boolean = false) = """
        {"tag_name":"$tag","draft":$draft,"prerelease":$prerelease,"assets":[${assets.joinToString(",")}]}
    """.trimIndent()

    /** Como a API devolve: uma release com outro prefixo de tag misturada às do app, fora de ordem. */
    private val sample = "[" + listOf(
        release("desktop-v0.13.0", listOf(asset("OpenObsidian-desktop.msi"), asset("outro.apk"))),
        release("v0.4.0", listOf(asset("LocalObsidian-Android-0.4.0.apk"))),
        release("v0.9.0", listOf(asset("LocalObsidian-Android-0.9.0.apk")), draft = true),
        release("v0.8.0", listOf(asset("LocalObsidian-Android-0.8.0.apk")), prerelease = true),
        release("v0.7.0", listOf(asset("notas.txt"), asset("LocalObsidian-Android-0.7.0.aab"))),
        release("v0.6.0-rc.1", listOf(asset("LocalObsidian-Android-0.6.0-rc.1.apk"))),
        release("v0.4.1", listOf(asset("LocalObsidian-Android-0.4.1.apk", digest = "sha256:" + hex.uppercase()))),
        release("v0.5.0", listOf(asset("SHA256SUMS"), asset("LocalObsidian-Android-0.5.0.apk", digest = "sha256:$hex"))),
        release("vlixo", listOf(asset("x.apk"))),
        release("v0.10.0", listOf(asset("evil.apk", url = "http://github.com/evil.apk"))),
        release("v0.11.0", listOf(asset("evil.apk", url = "https://example.com/evil.apk"))),
        release("v0.12.0", listOf(asset("evil.apk", url = "https://github.com/Anon5T4R/Local-Keys/releases/download/x/evil.apk"))),
    ).joinToString(",") + "]"

    @Test
    fun escolhe_a_android_mais_nova_publicada_com_apk() {
        val newest = ReleasePicker.newest(sample)!!
        assertEquals("0.5.0", newest.version)
        assertEquals("v0.5.0", newest.tag)
        assertEquals("LocalObsidian-Android-0.5.0.apk", newest.apkName)
        assertEquals("https://github.com/Anon5T4R/OpenObsidianAndroid/releases/download/x/LocalObsidian-Android-0.5.0.apk", newest.apkUrl)
        assertEquals(12345L, newest.apkSize)
        assertEquals(hex, newest.sha256)
    }

    @Test
    fun ignora_outro_prefixo_rascunho_pre_release_sem_apk_e_url_suspeita() {
        val versions = ReleasePicker.candidates(sample).map { it.version }
        assertEquals(listOf("0.5.0", "0.4.1", "0.4.0"), versions)
    }

    @Test
    fun compara_com_a_versao_instalada() {
        assertEquals("0.5.0", ReleasePicker.newerThan(sample, "0.4.1")?.version)
        assertNull(ReleasePicker.newerThan(sample, "0.5.0"))
        assertNull(ReleasePicker.newerThan(sample, "0.6.0"))
        // Pré-release instalada (build de teste) ainda recebe a final.
        assertEquals("0.5.0", ReleasePicker.newerThan(sample, "0.5.0-rc.1")?.version)
    }

    @Test
    fun lista_vazia_ou_so_outro_prefixo_nao_tem_atualizacao() {
        assertNull(ReleasePicker.newest("[]"))
        assertNull(ReleasePicker.newest("[" + release("desktop-v0.13.0", listOf(asset("a.apk"))) + "]"))
    }

    @Test
    fun json_invalido_vira_falha_de_parse() {
        listOf("", "{\"message\":\"API rate limit exceeded\"}", "<html>").forEach { body ->
            try {
                ReleasePicker.candidates(body)
                fail("esperava UpdateException para: $body")
            } catch (e: UpdateException) {
                assertEquals(UpdateFailure.PARSE, e.failure)
            }
        }
    }

    @Test
    fun release_guardada_vai_e_volta_e_a_corrompida_e_ignorada() {
        val r = ReleasePicker.newest(sample)!!
        assertEquals(r, AndroidRelease.fromJson(r.toJson()))
        val semDigest = r.copy(sha256 = null)
        assertEquals(semDigest, AndroidRelease.fromJson(semDigest.toJson()))
        assertNull(AndroidRelease.fromJson(null))
        assertNull(AndroidRelease.fromJson("{"))
        assertNull(AndroidRelease.fromJson(r.copy(apkUrl = "https://example.com/x.apk").toJson()))
        assertTrue(ReleasePicker.isAllowedDownload(r.apkUrl))
    }
}
