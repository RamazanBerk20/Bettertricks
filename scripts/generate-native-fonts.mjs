import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { WINETRICKS_BASELINE } from "./catalog-metadata.mjs";

const root = resolve(import.meta.dirname, "..");
const output = join(root, "catalog", "native", "fonts");
const marker = "# Generated from audited Winetricks font translations. Do not edit by hand.";

mkdirSync(output, { recursive: true });
for (const filename of readdirSync(output)) {
  const path = join(output, filename);
  if (filename.endsWith(".toml") && readFileSync(path, "utf8").startsWith(marker)) rmSync(path);
}

const coreFontRecipes = [
  {
    id: "andale",
    title: "MS Andale Mono font",
    files: [["andale32.exe", "https://github.com/pushcx/corefonts/raw/master/andale32.exe", "0524fe42951adc3a7eb870e32f0920313c71f170c859b5f770d82b4ee111e970"]],
    fonts: [["andalemo.ttf", "andalemo.ttf", "Andale Mono"]],
  },
  {
    id: "arial",
    title: "MS Arial / Arial Black fonts",
    files: [
      ["arial32.exe", "https://github.com/pushcx/corefonts/raw/master/arial32.exe", "85297a4d146e9c87ac6f74822734bdee5f4b2a722d7eaa584b7f2cbf76f478f6"],
      ["arialb32.exe", "https://github.com/pushcx/corefonts/raw/master/arialb32.exe", "a425f0ffb6a1a5ede5b979ed6177f4f4f4fdef6ae7c302a7b7720ef332fec0a8"],
    ],
    fonts: [
      ["arialbd.ttf", "arialbd.ttf", "Arial Bold"],
      ["arialbi.ttf", "arialbi.ttf", "Arial Bold Italic"],
      ["ariali.ttf", "ariali.ttf", "Arial Italic"],
      ["arial.ttf", "arial.ttf", "Arial"],
      ["ariblk.ttf", "ariblk.ttf", "Arial Black"],
    ],
  },
  {
    id: "comicsans",
    title: "MS Comic Sans fonts",
    files: [["comic32.exe", "https://github.com/pushcx/corefonts/raw/master/comic32.exe", "9c6df3feefde26d4e41d4a4fe5db2a89f9123a772594d7f59afd062625cd204e"]],
    fonts: [["comicbd.ttf", "comicbd.ttf", "Comic Sans MS Bold"], ["comic.ttf", "comic.ttf", "Comic Sans MS"]],
  },
  {
    id: "courier",
    title: "MS Courier fonts",
    files: [["courie32.exe", "https://github.com/pushcx/corefonts/raw/master/courie32.exe", "bb511d861655dde879ae552eb86b134d6fae67cb58502e6ff73ec5d9151f3384"]],
    fonts: [["courbd.ttf", "courbd.ttf", "Courier New Bold"], ["courbi.ttf", "courbi.ttf", "Courier New Bold Italic"], ["couri.ttf", "couri.ttf", "Courier New Italic"], ["cour.ttf", "cour.ttf", "Courier New"]],
  },
  {
    id: "georgia",
    title: "MS Georgia fonts",
    files: [["georgi32.exe", "https://github.com/pushcx/corefonts/raw/master/georgi32.exe", "2c2c7dcda6606ea5cf08918fb7cd3f3359e9e84338dc690013f20cd42e930301"]],
    fonts: [["georgiab.ttf", "georgiab.ttf", "Georgia Bold"], ["georgiai.ttf", "georgiai.ttf", "Georgia Italic"], ["georgia.ttf", "georgia.ttf", "Georgia"], ["georgiaz.ttf", "georgiaz.ttf", "Georgia Bold Italic"]],
  },
  {
    id: "impact",
    title: "MS Impact fonts",
    files: [["impact32.exe", "https://github.com/pushcx/corefonts/raw/master/impact32.exe", "6061ef3b7401d9642f5dfdb5f2b376aa14663f6275e60a51207ad4facf2fccfb"]],
    fonts: [["impact.ttf", "impact.ttf", "Impact"]],
  },
  {
    id: "times",
    title: "MS Times fonts",
    files: [["times32.exe", "https://github.com/pushcx/corefonts/raw/master/times32.exe", "db56595ec6ef5d3de5c24994f001f03b2a13e37cee27bc25c58f6f43e8f807ab"]],
    fonts: [["timesbd.ttf", "timesbd.ttf", "Times New Roman Bold"], ["timesbi.ttf", "timesbi.ttf", "Times New Roman Bold Italic"], ["timesi.ttf", "timesi.ttf", "Times New Roman Italic"], ["times.ttf", "times.ttf", "Times New Roman"]],
  },
  {
    id: "trebuchet",
    title: "MS Trebuchet fonts",
    files: [["trebuc32.exe", "https://github.com/pushcx/corefonts/raw/master/trebuc32.exe", "5a690d9bb8510be1b8b4fe49f1f2319651fe51bbe54775ddddd8ef0bd07fdac9"]],
    fonts: [["trebucbd.ttf", "trebucbd.ttf", "Trebuchet MS Bold"], ["trebucbi.ttf", "trebucbi.ttf", "Trebuchet MS Bold Italic"], ["trebucit.ttf", "trebucit.ttf", "Trebuchet MS Italic"], ["trebuc.ttf", "trebuc.ttf", "Trebuchet MS"]],
  },
  {
    id: "verdana",
    title: "MS Verdana fonts",
    files: [["verdan32.exe", "https://github.com/pushcx/corefonts/raw/master/verdan32.exe", "c1cb61255e363166794e47664e2f21af8e3a26cb6346eb8d2ae2fa85dd5aad96"]],
    fonts: [["verdanab.ttf", "verdanab.ttf", "Verdana Bold"], ["verdanai.ttf", "verdanai.ttf", "Verdana Italic"], ["verdana.ttf", "verdana.ttf", "Verdana"], ["verdanaz.ttf", "verdanaz.ttf", "Verdana Bold Italic"]],
  },
  {
    id: "webdings",
    title: "MS Webdings fonts",
    files: [["webdin32.exe", "https://github.com/pushcx/corefonts/raw/master/webdin32.exe", "64595b5abc1080fba8610c5c34fab5863408e806aafe84653ca8575bed17d75a"]],
    fonts: [["webdings.ttf", "webdings.ttf", "Webdings"]],
  },
];

const powerpointArchive = {
  id: "archive",
  filename: "PowerPointViewer.exe",
  cache_path: "PowerPointViewer/PowerPointViewer.exe",
  urls: ["https://web.archive.org/web/20171225132744if_/https://download.microsoft.com/download/E/6/7/E675FFFC-2A6D-4AB0-B3EB-27C9F8C8F696/PowerPointViewer.exe"],
  sha256: "249473568eba7a1e4f95498acba594e0f42e6581add4dead70c1dfb908a09423",
};

const powerpointFontRecipes = [
  {
    id: "calibri",
    title: "MS Calibri font",
    year: "2007",
    fonts: fontSet([
      ["CALIBRI.TTF", "calibri.ttf", "Calibri"],
      ["CALIBRIB.TTF", "calibrib.ttf", "Calibri Bold"],
      ["CALIBRII.TTF", "calibrii.ttf", "Calibri Italic"],
      ["CALIBRIZ.TTF", "calibriz.ttf", "Calibri Bold Italic"],
    ]),
  },
  {
    id: "cambria",
    title: "MS Cambria font",
    year: "2009",
    fonts: fontSet([
      ["CAMBRIA.TTC", "cambria.ttc", "Cambria & Cambria Math"],
      ["CAMBRIAB.TTF", "cambriab.ttf", "Cambria Bold"],
      ["CAMBRIAI.TTF", "cambriai.ttf", "Cambria Italic"],
      ["CAMBRIAZ.TTF", "cambriaz.ttf", "Cambria Bold Italic"],
    ]),
  },
  {
    id: "candara",
    title: "MS Candara font",
    year: "2009",
    fonts: fontSet([
      ["CANDARA.TTF", "candara.ttf", "Candara"],
      ["CANDARAB.TTF", "candarab.ttf", "Candara Bold"],
      ["CANDARAI.TTF", "candarai.ttf", "Candara Italic"],
      ["CANDARAZ.TTF", "candaraz.ttf", "Candara Bold Italic"],
    ]),
  },
  {
    id: "consolas",
    title: "MS Consolas console font",
    year: "2011",
    fonts: fontSet([
      ["CONSOLA.TTF", "consola.ttf", "Consolas"],
      ["CONSOLAB.TTF", "consolab.ttf", "Consolas Bold"],
      ["CONSOLAI.TTF", "consolai.ttf", "Consolas Italic"],
      ["CONSOLAZ.TTF", "consolaz.ttf", "Consolas Bold Italic"],
    ]),
  },
  {
    id: "constantia",
    title: "MS Constantia font",
    year: "2009",
    fonts: fontSet([
      ["CONSTAN.TTF", "constan.ttf", "Constantia"],
      ["CONSTANB.TTF", "constanb.ttf", "Constantia Bold"],
      ["CONSTANI.TTF", "constani.ttf", "Constantia Italic"],
      ["CONSTANZ.TTF", "constanz.ttf", "Constantia Bold Italic"],
    ]),
  },
  {
    id: "corbel",
    title: "MS Corbel font",
    year: "2009",
    fonts: fontSet([
      ["CORBEL.TTF", "corbel.ttf", "Corbel"],
      ["CORBELB.TTF", "corbelb.ttf", "Corbel Bold"],
      ["CORBELI.TTF", "corbeli.ttf", "Corbel Italic"],
      ["CORBELZ.TTF", "corbelz.ttf", "Corbel Bold Italic"],
    ]),
  },
  {
    id: "meiryo",
    title: "MS Meiryo font",
    year: "2009",
    conflicts: ["fakejapanese_vlgothic"],
    fonts: fontSet([
      ["MEIRYO.TTC", "meiryo.ttc", "Meiryo & Meiryo Italic & Meiryo UI & Meiryo UI Italic"],
      ["MEIRYOB.TTC", "meiryob.ttc", "Meiryo Bold & Meiryo Bold Italic & Meiryo UI Bold & Meiryo UI Bold Italic"],
    ]),
  },
];

const droidFiles = [
  ["DroidSans-Bold.ttf", "Droid Sans Bold", "2f529a3e60c007979d95d29794c3660694217fb882429fb33919d2245fe969e9"],
  ["DroidSansFallback.ttf", "Droid Sans Fallback", "05d71b179ef97b82cf1bb91cef290c600a510f77f39b4964359e3ef88378c79d"],
  ["DroidSansJapanese.ttf", "Droid Sans Japanese", "935867c21b8484c959170e62879460ae9363eae91f9b35e4519d24080e2eac30"],
  ["DroidSansMono.ttf", "Droid Sans Mono", "12b552de765dc1265d64f9f5566649930dde4dba07da0251d9f92801e70a1047"],
  ["DroidSans.ttf", "Droid Sans", "f51b88945f4c1b236f44b8d55a2d304316869127e95248c435c23f1e4142a7db"],
  ["DroidSerif-BoldItalic.ttf", "Droid Serif Bold Italic", "3fdf15b911c04317e5881ae1e4b9faefcdc4bf4cfb60223597d5c9455c3e4156"],
  ["DroidSerif-Bold.ttf", "Droid Serif Bold", "d28533eed8368f047eb5f57a88a91ba2ffc8b69a2dec5e50fe3f0c11ae3f4d8e"],
  ["DroidSerif-Italic.ttf", "Droid Serif Italic", "8a55a4823886234792991dd304dfa1fa120ae99483ec6c2255597d7d913b9a55"],
  ["DroidSerif-Regular.ttf", "Droid Serif", "22aea9471bea5bce1ec3bf7136c84f075b3d11cf09dffdc3dba05e570094cbde"],
];

const sourceHanDisplayNames = [
  ...["ExtraLight", "Light", "Normal", "", "Medium", "Bold", "Heavy"].map((weight) => `Source Han Sans SC${weight ? ` ${weight}` : ""}`),
  ...["ExtraLight", "Light", "Normal", "", "Medium", "Bold", "Heavy"].map((weight) => `Source Han Sans TC${weight ? ` ${weight}` : ""}`),
  ...["ExtraLight", "Light", "Normal", "", "Medium", "Bold", "Heavy"].map((weight) => `Source Han Sans${weight ? ` ${weight}` : ""}`),
  ...["ExtraLight", "Light", "Normal", "", "Medium", "Bold", "Heavy"].map((weight) => `Source Han Sans K${weight ? ` ${weight}` : ""}`),
];

const standaloneFontRecipes = [
  {
    id: "baekmuk",
    title: "Baekmuk Korean fonts",
    publisher: "Wooderart Inc. / kldp.net",
    year: "1999",
    files: [downloadFile("archive", "fonts-baekmuk_2.2.orig.tar.gz", "baekmuk/fonts-baekmuk_2.2.orig.tar.gz", "https://deb.debian.org/debian/pool/main/f/fonts-baekmuk/fonts-baekmuk_2.2.orig.tar.gz", "08ab7dffb55d5887cc942ce370f5e33b756a55fbb4eaf0b90f244070e8d51882")],
    extracts: [archiveExtract("archive", "tar_gz", ["baekmuk-ttf-2.2/ttf"])],
    source: "${temp}/baekmuk/baekmuk-ttf-2.2/ttf",
    fonts: sameNameFonts([["batang.ttf", "Baekmuk Batang"], ["gulim.ttf", "Baekmuk Gulim"], ["dotum.ttf", "Baekmuk Dotum"], ["hline.ttf", "Baekmuk Headline"]]),
  },
  {
    id: "droid",
    title: "Droid fonts",
    publisher: "Ascender Corporation",
    year: "2009",
    files: droidFiles.map(([filename, , sha256], index) => downloadFile(`font_${index + 1}`, filename, `droid/${filename}`, `https://raw.githubusercontent.com/android/platform_frameworks_base/feef9887e8f8eb6f64fc1b4552c02efb5755cdc1/data/fonts/${filename}`, sha256)),
    extracts: [],
    source: "${cache}/droid",
    fonts: droidFiles.map(([filename, display_name]) => ({ source: filename, filename: filename.toLowerCase(), display_name })),
  },
  {
    id: "eufonts",
    title: "Updated fonts for Romanian and Bulgarian",
    publisher: "Microsoft",
    year: "2008",
    files: [downloadFile("archive", "EUupdate.EXE", "eufonts/EUupdate.EXE", "https://sourceforge.net/projects/mscorefonts2/files/cabs/EUupdate.EXE", "464dd2cd5f09f489f9ac86ea7790b7b8548fc4e46d9f889b68d2cdce47e09ea8")],
    extracts: [archiveExtract("archive", "cabinet")],
    source: "${temp}/eufonts",
    fonts: sameNameFonts([
      ["arialbd.ttf", "Arial Bold"], ["arialbi.ttf", "Arial Bold Italic"], ["ariali.ttf", "Arial Italic"], ["arial.ttf", "Arial"],
      ["timesbd.ttf", "Times New Roman Bold"], ["timesbi.ttf", "Times New Roman Bold Italic"], ["timesi.ttf", "Times New Roman Italic"], ["times.ttf", "Times New Roman"],
      ["trebucbd.ttf", "Trebuchet MS Bold"], ["trebucbi.ttf", "Trebuchet MS Bold Italic"], ["trebucit.ttf", "Trebuchet MS Italic"], ["trebuc.ttf", "Trebuchet MS"],
      ["verdanab.ttf", "Verdana Bold"], ["verdanai.ttf", "Verdana Italian"], ["verdana.ttf", "Verdana"], ["verdanaz.ttf", "Verdana Bold Italic"],
    ]),
  },
  {
    id: "ipamona",
    title: "IPAMona Japanese fonts",
    publisher: "Jun Kobayashi",
    year: "2008",
    files: [downloadFile("archive", "opfc-ModuleHP-1.1.1_withIPAMonaFonts-1.0.8.tar.gz", "ipamona/opfc-ModuleHP-1.1.1_withIPAMonaFonts-1.0.8.tar.gz", "https://web.archive.org/web/20190309175311if_/http://www.geocities.jp/ipa_mona/opfc-ModuleHP-1.1.1_withIPAMonaFonts-1.0.8.tar.gz", "ab77beea3b051abf606cd8cd3badf6cb24141ef145c60f508fcfef1e3852bb9d")],
    extracts: [archiveExtract("archive", "tar_gz", ["opfc-ModuleHP-1.1.1_withIPAMonaFonts-1.0.8/fonts"])],
    source: "${temp}/ipamona/opfc-ModuleHP-1.1.1_withIPAMonaFonts-1.0.8/fonts",
    fonts: sameNameFonts([["ipagui-mona.ttf", "IPAMonaUIGothic"], ["ipag-mona.ttf", "IPAMonaGothic"], ["ipagp-mona.ttf", "IPAMonaPGothic"], ["ipam-mona.ttf", "IPAMonaMincho"], ["ipamp-mona.ttf", "IPAMonaPMincho"]]),
  },
  {
    id: "liberation",
    title: "Red Hat Liberation fonts (Mono, Sans, SansNarrow, Serif)",
    publisher: "Red Hat",
    year: "2008",
    files: [downloadFile("archive", "liberation-fonts-ttf-1.07.4.tar.gz", "liberation/liberation-fonts-ttf-1.07.4.tar.gz", "https://releases.pagure.org/liberation-fonts/liberation-fonts-ttf-1.07.4.tar.gz", "61a7e2b6742a43c73e8762cdfeaf6dfcf9abdd2cfa0b099a9854d69bc4cfee5c")],
    extracts: [archiveExtract("archive", "tar_gz")],
    source: "${temp}/liberation/liberation-fonts-ttf-1.07.4",
    fonts: sameNameFonts([
      ["liberationmono-bolditalic.ttf", "Liberation Mono Bold Italic"], ["liberationmono-bold.ttf", "Liberation Mono Bold"], ["liberationmono-italic.ttf", "Liberation Mono Italic"], ["liberationmono-regular.ttf", "Liberation Mono"],
      ["liberationsans-bolditalic.ttf", "Liberation Sans Bold Italic"], ["liberationsans-bold.ttf", "Liberation Sans Bold"], ["liberationsans-italic.ttf", "Liberation Sans Italic"], ["liberationsans-regular.ttf", "Liberation Sans"],
      ["liberationsansnarrow-bolditalic.ttf", "Liberation Sans Narrow Bold Italic"], ["liberationsansnarrow-bold.ttf", "Liberation Sans Narrow Bold"], ["liberationsansnarrow-italic.ttf", "Liberation Sans Narrow Italic"], ["liberationsansnarrow-regular.ttf", "Liberation Sans Narrow"],
      ["liberationserif-bolditalic.ttf", "Liberation Serif Bold Italic"], ["liberationserif-bold.ttf", "Liberation Serif Bold"], ["liberationserif-italic.ttf", "Liberation Serif Italic"], ["liberationserif-regular.ttf", "Liberation Serif"],
    ]),
  },
  {
    id: "lucida",
    title: "MS Lucida Console font",
    publisher: "Microsoft",
    year: "1998",
    files: [downloadFile("archive", "eurofixi.exe", "lucida/eurofixi.exe", "https://ftpmirror.your.org/pub/misc/ftp.microsoft.com/bussys/winnt/winnt-public/fixes/usa/NT40TSE/hotfixes-postSP3/Euro-fix/eurofixi.exe", "41f272a33521f6e15f2cce9ff1e049f2badd5ff0dc327fc81b60825766d5b6c7")],
    extracts: [archiveExtract("archive", "cabinet", ["lucon.ttf"])],
    source: "${temp}/lucida",
    fonts: sameNameFonts([["lucon.ttf", "Lucida Console"]]),
  },
  {
    id: "opensymbol",
    title: "OpenSymbol fonts (replacement for Wingdings)",
    publisher: "libreoffice.org",
    year: "2022",
    files: [downloadFile("font", "opens___.ttf", "opensymbol/opens___.ttf", "https://raw.githubusercontent.com/apache/openoffice/5f13fa00702a0abe48858d443bc306f5c5ba26d8/main/extras/source/truetype/symbol/opens___.ttf", "86f6a40ca61adfc5942fb4d2fc360ffba9abd972a7e21c1ee91e494299ff0cbc")],
    extracts: [],
    source: "${cache}/opensymbol",
    fonts: sameNameFonts([["opens___.ttf", "OpenSymbol"]]),
  },
  {
    id: "sourcehansans",
    title: "Source Han Sans fonts",
    publisher: "Adobe",
    year: "2021",
    files: [downloadFile("archive", "SourceHanSans.ttc.zip", "sourcehansans/SourceHanSans.ttc.zip", "https://github.com/adobe-fonts/source-han-sans/releases/download/2.004R/SourceHanSans.ttc.zip", "6f59118a9adda5a7fe4e9e6bb538309f7e1d3c5411f9a9d32af32a79501b7e4f")],
    extracts: [archiveExtract("archive", "zip", ["SourceHanSans.ttc"])],
    source: "${temp}/sourcehansans",
    fonts: sourceHanDisplayNames.map((display_name) => ({ source: "SourceHanSans.ttc", filename: "sourcehansans.ttc", display_name })),
  },
  {
    id: "tahoma",
    title: "MS Tahoma font (not part of corefonts)",
    publisher: "Microsoft",
    year: "1999",
    files: [downloadFile("archive", "IELPKTH.CAB", "tahoma/IELPKTH.CAB", "https://downloads.sourceforge.net/corefonts/OldFiles/IELPKTH.CAB", "c1be3fb8f0042570be76ec6daa03a99142c88367c1bc810240b85827c715961a")],
    extracts: [archiveExtract("archive", "cabinet", ["*.TTF"])],
    source: "${temp}/tahoma",
    fonts: sameNameFonts([["tahoma.ttf", "Tahoma"], ["tahomabd.ttf", "Tahoma Bold"]]),
  },
  {
    id: "takao",
    title: "Takao Japanese fonts",
    publisher: "Jun Kobayashi",
    year: "2010",
    files: [downloadFile("archive", "takao-fonts-ttf-003.02.01.zip", "takao/takao-fonts-ttf-003.02.01.zip", "https://launchpad.net/takao-fonts/trunk/003.02.01/+download/takao-fonts-ttf-003.02.01.zip", "2f526a16c7931958f560697d494d8304949b3ce0aef246fb0c727fbbcc39089e")],
    extracts: [archiveExtract("archive", "zip")],
    source: "${temp}/takao/takao-fonts-ttf-003.02.01",
    fonts: sameNameFonts([["takaogothic.ttf", "TakaoGothic"], ["takaopgothic.ttf", "TakaoPGothic"], ["takaomincho.ttf", "TakaoMincho"], ["takaopmincho.ttf", "TakaoPMincho"], ["takaoexgothic.ttf", "TakaoExGothic"], ["takaoexmincho.ttf", "TakaoExMincho"]]),
  },
  {
    id: "uff",
    title: "Ubuntu Font Family",
    publisher: "Ubuntu",
    year: "2010",
    files: [downloadFile("archive", "ubuntu-font-family-0.83.zip", "uff/ubuntu-font-family-0.83.zip", "https://assets.ubuntu.com/v1/fad7939b-ubuntu-font-family-0.83.zip", "456d7d42797febd0d7d4cf1b782a2e03680bb4a5ee43cc9d06bda172bac05b42")],
    extracts: [archiveExtract("archive", "zip")],
    source: "${temp}/uff/ubuntu-font-family-0.83",
    fonts: sameNameFonts([["ubuntu-bi.ttf", "Ubuntu Bold Italic"], ["ubuntu-b.ttf", "Ubuntu Bold"], ["ubuntu-c.ttf", "Ubuntu Condensed"], ["ubuntu-i.ttf", "Ubuntu Italic"], ["ubuntu-li.ttf", "Ubuntu Light Italic"], ["ubuntu-l.ttf", "Ubuntu Light"], ["ubuntu-mi.ttf", "Ubuntu Medium Italic"], ["ubuntumono-bi.ttf", "Ubuntu Mono Bold Italic"], ["ubuntumono-b.ttf", "Ubuntu Mono Bold"], ["ubuntumono-ri.ttf", "Ubuntu Mono Italic"], ["ubuntumono-r.ttf", "Ubuntu Mono"], ["ubuntu-m.ttf", "Ubuntu Medium"], ["ubuntu-ri.ttf", "Ubuntu Italic"], ["ubuntu-r.ttf", "Ubuntu"]]),
  },
  {
    id: "vlgothic",
    title: "VLGothic Japanese fonts",
    publisher: "Project Vine / Daisuke Suzuki",
    year: "2014",
    files: [downloadFile("archive", "VLGothic-20141206.tar.xz", "vlgothic/VLGothic-20141206.tar.xz", "https://mirrors.gigenet.com/OSDN/vlgothic/62375/VLGothic-20141206.tar.xz", "982040db2f9cb73d7c6ab7d9d163f2ed46d1180f330c9ba2fae303649bf8102d")],
    extracts: [archiveExtract("archive", "tar_xz")],
    source: "${temp}/vlgothic/VLGothic",
    fonts: sameNameFonts([["vl-gothic-regular.ttf", "VL Gothic"], ["vl-pgothic-regular.ttf", "VL PGothic"]]),
  },
  {
    id: "wenquanyi",
    title: "WenQuanYi CJK font",
    publisher: "wenq.org",
    year: "2009",
    files: [downloadFile("archive", "wqy-microhei-0.2.0-beta.tar.gz", "wenquanyi/wqy-microhei-0.2.0-beta.tar.gz", "https://downloads.sourceforge.net/wqy/wqy-microhei-0.2.0-beta.tar.gz", "2802ac8023aa36a66ea6e7445854e3a078d377ffff42169341bd237871f7213e")],
    extracts: [archiveExtract("archive", "tar_gz")],
    source: "${temp}/wenquanyi/wqy-microhei",
    fonts: sameNameFonts([["wqy-microhei.ttc", "WenQuanYi Micro Hei"]]),
  },
  {
    id: "wenquanyizenhei",
    title: "WenQuanYi ZenHei font",
    publisher: "wenq.org",
    year: "2009",
    files: [downloadFile("archive", "wqy-zenhei-0.8.38-1.tar.gz", "wenquanyizenhei/wqy-zenhei-0.8.38-1.tar.gz", "https://downloads.sourceforge.net/wqy/wqy-zenhei-0.8.38-1.tar.gz", "6018eb54243eddc41e9cbe0b71feefa5cb2570ecbaccd39daa025961235dea22")],
    extracts: [archiveExtract("archive", "tar_gz")],
    source: "${temp}/wenquanyizenhei/wqy-zenhei",
    fonts: sameNameFonts([["wqy-zenhei.ttc", "WenQuanYi Zen Hei"]]),
  },
  {
    id: "unifont",
    title: "Unifont alternative to Arial Unicode MS",
    publisher: "Roman Czyborra / GNU",
    year: "2021",
    files: [downloadFile("font", "unifont-13.0.06.ttf", "unifont/unifont-13.0.06.ttf", "https://unifoundry.com/pub/unifont/unifont-13.0.06/font-builds/unifont-13.0.06.ttf", "d73c0425811ffd366b0d1973e9338bac26fe7cf085760a12e10c61241915e742")],
    extracts: [],
    source: "${cache}/unifont",
    fonts: fontSet([["unifont-13.0.06.ttf", "unifont.ttf", "Unifont"]]),
    replacements: [{ alias: "Arial Unicode MS", replacement: "Unifont" }],
  },
];

const replacementRecipes = [
  {
    id: "fakechinese",
    title: "Creates aliases for Chinese fonts using Source Han Sans fonts",
    publisher: "Adobe",
    year: "2019",
    dependencies: ["sourcehansans"],
    replacements: [
      ...["Dengxian", "FangSong", "KaiTi", "Microsoft YaHei", "Microsoft YaHei UI", "NSimSun", "SimHei", "SimKai", "SimSun", "SimSun-ExtB"].map((alias) => ({ alias, replacement: "Source Han Sans SC" })),
      ...["DFKai-SB", "Microsoft JhengHei", "Microsoft JhengHei UI", "MingLiU", "PMingLiU", "MingLiU-ExtB", "PMingLiU-ExtB"].map((alias) => ({ alias, replacement: "Source Han Sans TC" })),
    ],
  },
  {
    id: "fakejapanese",
    title: "Creates aliases for Japanese fonts using Source Han Sans fonts",
    publisher: "Adobe",
    year: "2019",
    dependencies: ["sourcehansans"],
    replacements: ["Meiryo", "Meiryo UI", "MS Gothic", "MS PGothic", "MS Mincho", "MS PMincho", "MS UI Gothic", "UD Digi KyoKasho N-R", "UD Digi KyoKasho NK-R", "UD Digi KyoKasho NP-R", "Yu Gothic", "Yu Gothic UI", "Yu Mincho", "メイリオ", "ＭＳ ゴシック", "ＭＳ Ｐゴシック", "ＭＳ 明朝", "ＭＳ Ｐ明朝"].map((alias) => ({ alias, replacement: "Source Han Sans" })),
  },
  {
    id: "fakejapanese_ipamona",
    title: "Creates aliases for Japanese fonts using IPAMona fonts",
    publisher: "Jun Kobayashi",
    year: "2008",
    dependencies: ["ipamona"],
    replacements: [
      ["MS UI Gothic", "IPAMonaUIGothic"], ["MS Gothic", "IPAMonaGothic"], ["MS PGothic", "IPAMonaPGothic"], ["MS Mincho", "IPAMonaMincho"], ["MS PMincho", "IPAMonaPMincho"],
      ["ＭＳ ゴシック", "IPAMonaGothic"], ["ＭＳ Ｐゴシック", "IPAMonaPGothic"], ["ＭＳ 明朝", "IPAMonaMincho"], ["ＭＳ Ｐ明朝", "IPAMonaPMincho"],
    ].map(([alias, replacement]) => ({ alias, replacement })),
  },
  {
    id: "fakejapanese_vlgothic",
    title: "Creates aliases for Japanese Meiryo fonts using VLGothic fonts",
    publisher: "Project Vine / Daisuke Suzuki",
    year: "2014",
    dependencies: ["vlgothic"],
    conflicts: ["meiryo"],
    replacements: ["Meiryo UI", "Meiryo", "メイリオ"].map((alias) => ({ alias, replacement: "VL Gothic" })),
  },
  {
    id: "fakekorean",
    title: "Creates aliases for Korean fonts using Source Han Sans fonts",
    publisher: "Adobe",
    year: "2019",
    dependencies: ["sourcehansans"],
    replacements: ["Batang", "BatangChe", "Dotum", "DotumChe", "Gulim", "GulimChe", "Gungsuh", "GungsuhChe", "Malgun Gothic", "바탕", "바탕체", "돋움", "돋움체", "굴림", "굴림체", "맑은 고딕"].map((alias) => ({ alias, replacement: "Source Han Sans K" })),
  },
];

for (const recipe of coreFontRecipes) writeFontRecipe(recipe);
for (const recipe of powerpointFontRecipes) writePowerpointFontRecipe(recipe);
for (const recipe of standaloneFontRecipes) writeStandaloneFontRecipe(recipe);
for (const recipe of replacementRecipes) writeReplacementRecipe(recipe);

writeRecipe({
  id: "corefonts",
  title: "MS Arial, Courier, Times fonts",
  description: "Install the complete Microsoft core-font collection used by upstream Winetricks.",
  publisher: "Microsoft",
  year: "2008",
  media: "download",
  dependencies: coreFontRecipes.map((recipe) => recipe.id),
  files: [],
  detect: [{ path: "${windows}/Fonts/corefonts.installed", kind: "file" }],
  steps: [{ type: "ensure_file", path: "${windows}/Fonts/corefonts.installed" }],
  verify: [{ type: "verify_path", path: "${windows}/Fonts/corefonts.installed", kind: "file" }],
});

writeAggregateRecipe({
  id: "pptfonts",
  title: "All MS PowerPoint Viewer fonts",
  publisher: "various",
  year: "2007-2009",
  dependencies: powerpointFontRecipes.map((recipe) => recipe.id),
});

writeAggregateRecipe({
  id: "cjkfonts",
  title: "All Chinese, Japanese, Korean fonts and aliases",
  publisher: "Various",
  year: "1999-2019",
  dependencies: ["fakechinese", "fakejapanese", "fakekorean", "unifont"],
});

const generatedCount = coreFontRecipes.length + 1 + powerpointFontRecipes.length + standaloneFontRecipes.length + replacementRecipes.length + 2;
process.stdout.write(`Generated ${generatedCount} native font recipes.\n`);

function writeFontRecipe(recipe) {
  const directory = `\${temp}/${recipe.id}`;
  writeRecipe({
    ...recipe,
    description: `Install and register ${recipe.title.replace(/^MS /, "Microsoft ")} in the selected prefix.`,
    publisher: "Microsoft",
    year: "2008",
    media: "download",
    dependencies: [],
    files: recipe.files.map(([filename, url, sha256], index) => ({
      id: `archive_${index + 1}`,
      filename,
      cache_path: `corefonts/${filename}`,
      urls: [url],
      sha256,
    })),
    detect: [{ path: `\${windows}/Fonts/${recipe.fonts.at(-1)[1]}`, kind: "file" }],
    steps: [
      ...recipe.files.map((_, index) => ({ type: "download", file: `archive_${index + 1}` })),
      { type: "ensure_directory", path: directory },
      ...recipe.files.map((_, index) => ({ type: "extract", file: `archive_${index + 1}`, destination: directory, format: "cabinet", include: [] })),
      { type: "install_fonts", source: directory, fonts: recipe.fonts.map(([source, filename, display_name]) => ({ source, filename, display_name })) },
    ],
    verify: recipe.fonts.map(([, filename]) => ({ type: "verify_path", path: `\${windows}/Fonts/${filename}`, kind: "file" })),
  });
}

function writePowerpointFontRecipe(recipe) {
  const directory = `\${temp}/${recipe.id}`;
  writeRecipe({
    ...recipe,
    description: `Install and register ${recipe.title.replace(/^MS /, "Microsoft ")} from the pinned PowerPoint Viewer archive.`,
    publisher: "Microsoft",
    media: "download",
    dependencies: [],
    conflicts: recipe.conflicts ?? [],
    files: [{ ...powerpointArchive }],
    detect: [{ path: `\${windows}/Fonts/${recipe.fonts[0].filename}`, kind: "file" }],
    steps: [
      { type: "download", file: "archive" },
      { type: "ensure_directory", path: directory },
      { type: "extract", file: "archive", destination: directory, format: "cabinet", include: ["ppviewer.cab"] },
      { type: "extract_path", source: `${directory}/ppviewer.cab`, destination: directory, format: "cabinet", include: recipe.fonts.map((font) => font.source) },
      { type: "install_fonts", source: directory, fonts: recipe.fonts },
    ],
    verify: recipe.fonts.map((font) => ({ type: "verify_path", path: `\${windows}/Fonts/${font.filename}`, kind: "file" })),
  });
}

function writeStandaloneFontRecipe(recipe) {
  const directory = `\${temp}/${recipe.id}`;
  const installedFiles = [...new Set(recipe.fonts.map((font) => font.filename))];
  const extractionSteps = recipe.extracts.flatMap((extract) => [
    {
      ...extract,
      destination: extract.destination ?? directory,
    },
  ]);
  writeRecipe({
    ...recipe,
    description: `Install and register ${recipe.title} in the selected prefix.`,
    media: "download",
    dependencies: recipe.dependencies ?? [],
    conflicts: recipe.conflicts ?? [],
    detect: [{ path: `\${windows}/Fonts/${recipe.fonts.at(-1).filename}`, kind: "file" }],
    steps: [
      ...recipe.files.map((file) => ({ type: "download", file: file.id })),
      ...(extractionSteps.length ? [{ type: "ensure_directory", path: directory }] : []),
      ...extractionSteps,
      { type: "install_fonts", source: recipe.source, fonts: recipe.fonts },
      ...(recipe.replacements?.length ? [{ type: "font_replacements", replacements: recipe.replacements }] : []),
    ],
    verify: installedFiles.map((filename) => ({ type: "verify_path", path: `\${windows}/Fonts/${filename}`, kind: "file" })),
  });
}

function writeReplacementRecipe(recipe) {
  writeRecipe({
    ...recipe,
    description: `${recipe.title}, matching the pinned Winetricks replacement table.`,
    media: "none",
    dependencies: recipe.dependencies,
    conflicts: recipe.conflicts ?? [],
    files: [],
    detect: [],
    steps: [{ type: "font_replacements", replacements: recipe.replacements }],
    verify: [],
  });
}

function writeAggregateRecipe(recipe) {
  writeRecipe({
    ...recipe,
    description: `Install every recipe in the upstream ${recipe.title} aggregate.`,
    media: "download",
    conflicts: [],
    files: [],
    detect: [],
    steps: [{ type: "native_action", action: "noop" }],
    verify: [],
  });
}

function downloadFile(id, filename, cache_path, url, sha256) {
  return { id, filename, cache_path, urls: [url], sha256 };
}

function archiveExtract(file, format, include = []) {
  return { type: "extract", file, format, include };
}

function fontSet(fonts) {
  return fonts.map(([source, filename, display_name]) => ({ source, filename, display_name }));
}

function sameNameFonts(fonts) {
  return fonts.map(([filename, display_name]) => ({ source: filename, filename, display_name }));
}

function writeRecipe(recipe) {
  const lines = [
    marker,
    "schema = 1",
    `id = ${JSON.stringify(recipe.id)}`,
    'category = "fonts"',
    `title = ${JSON.stringify(recipe.title)}`,
    `publisher = ${JSON.stringify(recipe.publisher)}`,
    `year = ${JSON.stringify(recipe.year)}`,
    `description = ${JSON.stringify(recipe.description)}`,
    `media = ${JSON.stringify(recipe.media)}`,
    'maturity = "native"',
    'tags = ["fonts", "compatibility"]',
  ];
  if (recipe.dependencies.length) lines.push(`dependencies = [${recipe.dependencies.map(JSON.stringify).join(", ")}]`);
  if (recipe.conflicts?.length) lines.push(`conflicts = [${recipe.conflicts.map((value) => JSON.stringify(value)).join(", ")}]`);
  for (const file of recipe.files) {
    lines.push("", "[[files]]", `id = ${JSON.stringify(file.id)}`, `filename = ${JSON.stringify(file.filename)}`, `cache_path = ${JSON.stringify(file.cache_path)}`, `urls = [${file.urls.map(JSON.stringify).join(", ")}]`, `sha256 = ${JSON.stringify(file.sha256)}`, "manual = false");
  }
  for (const detect of recipe.detect) lines.push("", "[[detect]]", `path = ${JSON.stringify(detect.path)}`, `kind = ${JSON.stringify(detect.kind)}`);
  for (const step of recipe.steps) writeStep(lines, "steps", step);
  for (const step of recipe.verify) writeStep(lines, "verify", step);
  lines.push("", "[source]", `upstream_tag = ${JSON.stringify(WINETRICKS_BASELINE)}`, `upstream_verb = ${JSON.stringify(recipe.id)}`, "");
  writeFileSync(join(output, `${recipe.id}.toml`), lines.join("\n"));
}

function writeStep(lines, table, step) {
  lines.push("", `[[${table}]]`, `type = ${JSON.stringify(step.type)}`);
  for (const [key, value] of Object.entries(step)) {
    if (key === "type") continue;
    if (key === "fonts") {
      lines.push(`fonts = [${value.map((font) => `{ source = ${JSON.stringify(font.source)}, filename = ${JSON.stringify(font.filename)}, display_name = ${JSON.stringify(font.display_name)} }`).join(", ")}]`);
    } else if (key === "replacements") {
      lines.push(`replacements = [${value.map((replacement) => `{ alias = ${JSON.stringify(replacement.alias)}, replacement = ${JSON.stringify(replacement.replacement)} }`).join(", ")}]`);
    } else if (Array.isArray(value)) {
      lines.push(`${key} = [${value.map((item) => JSON.stringify(item)).join(", ")}]`);
    } else {
      lines.push(`${key} = ${JSON.stringify(value)}`);
    }
  }
}
