<?xml version="1.0"?>
<!--
  CEL variant of the W3C SCXML IRP templates, for Vertep's `cel` datamodel (vertep-dm-cel).

  Modeled on confEcma.xsl (Copyright 1998-2003 W3C (MIT, ERCIM, Keio), All Rights Reserved.
  See http://www.w3.org/Consortium/Legal/), template for template, with the conf: attributes
  mapped to CEL:

  - a data item `Var<n>` is the context field `ctx.Var<n>`: ids, names, locations, `item`,
    `index`, `idlocation` and `namelist` keep the bare name (a bare location means a field of
    ctx), while every expression reads it as `ctx.Var<n>`;
  - `_event`, `_sessionid`, `_name`, `_ioprocessors` and `In(id)` are bound as in SCXML;
  - an unbound data item is `null`, so "is bound" is `!= null` and "has no value" `== null`;
  - guards must have type `bool`, so ECMAScript truthiness tests become explicit comparisons;
  - ECMAScript strict equality `===` is CEL `==`.

  Templates that only make sense in ECMAScript (`<script>`, and the regular expressions over
  `_event.raw` of the Basic HTTP tests) are kept so that every template transforms, but the
  import script does not generate CEL variants of the tests that use them
  (see scripts/import-irp.sh and suite/cel/EXCLUDED.tsv).
-->
<xsl:stylesheet
    xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:conf="http://www.w3.org/2005/scxml-conformance"
    version="2.0">

<!-- Copy everything that doesn't match other rules -->
<xsl:template match="/ | @* | node()">
  <xsl:copy>
    <xsl:apply-templates select="@* | node()"/>
  </xsl:copy>
</xsl:template>

<!-- Success criteria -->

<xsl:template match="//@conf:targetpass">
	<xsl:attribute name="target">pass</xsl:attribute>
</xsl:template>

<xsl:template match="conf:pass">
 <final xmlns="http://www.w3.org/2005/07/scxml" id="pass">
   <onentry>
     <log label="Outcome" expr="'pass'"/>
   </onentry>
 </final>
</xsl:template>

<!-- Failure criteria -->

<xsl:template match="//@conf:targetfail">
	<xsl:attribute name="target">fail</xsl:attribute>
</xsl:template>

<xsl:template match="conf:fail">
 <final xmlns="http://www.w3.org/2005/07/scxml" id="fail">
   <onentry>
    <log label="Outcome" expr="'fail'"/>
   </onentry>
</final>
</xsl:template>

<!-- datamodel -->
<xsl:template match="//@conf:datamodel">
	<xsl:attribute name="datamodel">cel</xsl:attribute>
</xsl:template>

<!-- creates id for <data> element, etc. (a field of ctx) -->
<xsl:template match="//@conf:id">
	<xsl:attribute name="id">Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- creates name for <param>, etc. -->
<xsl:template match="//@conf:name">
	<xsl:attribute name="name">Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- creates location for <assign>, etc.: a bare location is a field of ctx -->
<xsl:template match="//@conf:location">
	<xsl:attribute name="location">Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- names an invalid location for <assign>, etc. -->
<xsl:template match="//@conf:invalidLocation">
	<xsl:attribute name="location">foo.bar.baz </xsl:attribute>
</xsl:template>

<!-- the location of a system variable (read-only) -->
<xsl:template match="//@conf:systemVarLocation">
	<xsl:attribute name="location"><xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- expr is evaluated -->
<xsl:template match="//@conf:expr">
	<xsl:attribute name="expr"><xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- targetexpr is the corresponding ID -->
<xsl:template match="//@conf:targetVar">
	<xsl:attribute name="targetexpr">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- expr is quoted -->
<xsl:template match="//@conf:quoteExpr">
	<xsl:attribute name="expr">'<xsl:value-of select="." />'</xsl:attribute>
</xsl:template>

<!-- an expr that is the value of a variable -->
<xsl:template match="//@conf:varExpr">
	<xsl:attribute name="expr">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- the same as varExpr -->
<xsl:template match="//@conf:varChildExpr">
	<xsl:attribute name="expr">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- an expr that is the value of a system variable -->
<xsl:template match="//@conf:systemVarExpr">
	<xsl:attribute name="expr"><xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- an expr that is the value of a non-existent substructure of a variable -->
<xsl:template match="//@conf:varNonexistentStruct">
	<xsl:attribute name="expr">ctx.Var<xsl:value-of select="." />.bar</xsl:attribute>
</xsl:template>

<!-- an expression that cannot be evaluated (a CEL syntax error) -->
<xsl:template match="//@conf:illegalExpr">
	<xsl:attribute name="expr">1 +</xsl:attribute>
</xsl:template>

<!-- add 1 to the value of the specified variable -->
<xsl:template match="conf:incrementID">
	<assign xmlns="http://www.w3.org/2005/07/scxml">
	  <xsl:attribute name="location">Var<xsl:value-of select="@id"/></xsl:attribute>
	  <xsl:attribute name="expr">ctx.Var<xsl:value-of select="@id"/> + 1</xsl:attribute>
	  </assign>
	</xsl:template>

<!-- concatenate the value of id2 to id1 and assign the result to id1 -->
<xsl:template match="conf:concatVars">
	<assign xmlns="http://www.w3.org/2005/07/scxml">
	  <xsl:attribute name="location">Var<xsl:value-of select="@id1"/></xsl:attribute>
	  <xsl:attribute name="expr">ctx.Var<xsl:value-of select="@id1"/> + ctx.Var<xsl:value-of select="@id2"/></xsl:attribute>
	  </assign>
	</xsl:template>

<!-- assign the sum of the values of two vars to the first var -->
<xsl:template match="//conf:sumVars">
	<assign xmlns="http://www.w3.org/2005/07/scxml">
<xsl:attribute name="location">Var<xsl:value-of select="@id1"/></xsl:attribute>
<xsl:attribute name="expr">ctx.Var<xsl:value-of select="@id1"/> + ctx.Var<xsl:value-of select="@id2"/></xsl:attribute>
	  </assign>
	</xsl:template>

<!-- an illegal target for <send> causing a send error to be raised -->
<xsl:template match="//@conf:illegalTarget">
	<xsl:attribute name="target">baz</xsl:attribute>
</xsl:template>

<!-- a legal, but unreachable, target for <send> causing a send error to be raised -->
<xsl:template match="//@conf:unreachableTarget">
	<xsl:attribute name="target">#_scxml_foo</xsl:attribute>
</xsl:template>

<!-- illegal content for <send> causing the message to be rejected -->
<xsl:template match="//conf:illegalContent">
	<content xmlns="http://www.w3.org/2005/07/scxml"> xyz </content>
</xsl:template>

<!-- a content element whose value is the string 'foo' -->
<xsl:template match="//conf:contentFoo">
	<content xmlns="http://www.w3.org/2005/07/scxml">foo</content>
</xsl:template>

<xsl:template match="//conf:someInlineVal">123</xsl:template>

<!-- something that is guaranteed not to be the ID of the current session -->
<xsl:template match="//@conf:invalidSessionID">
	<xsl:attribute name="expr">27</xsl:attribute>
</xsl:template>

<!-- something that is guaranteed not to be a valid event I/O processor -->
<xsl:template match="//@conf:invalidSendType">
	<xsl:attribute name="type">27</xsl:attribute>
</xsl:template>

<!-- same value in an expr -->
<xsl:template match="//@conf:invalidSendTypeExpr">
	<xsl:attribute name="expr">27</xsl:attribute>
</xsl:template>

<!-- exprs that return the value of the event fields -->

<xsl:template match="//@conf:eventName">
	<xsl:attribute name="expr">_event.name</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:eventType">
	<xsl:attribute name="expr">_event.type</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:eventSendid">
	<xsl:attribute name="expr">_event.sendid</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:eventField">
	<xsl:attribute name="expr">_event.<xsl:value-of select="."/></xsl:attribute>
</xsl:template>

<!-- the raw message structure: not part of the CEL event (tests using it are not generated) -->
<xsl:template match="//@conf:eventRaw">
	<xsl:attribute name="expr">_event.raw</xsl:attribute>
</xsl:template>

<!-- the value of the specified item in _event.data -->
<xsl:template match="//@conf:eventDataFieldValue">
	<xsl:attribute name="expr">_event.data.<xsl:value-of select="."/></xsl:attribute>
</xsl:template>

<!-- the value of a KVP specified by <param> from _event.data -->
<xsl:template match="//@conf:eventDataParamValue">
	<xsl:attribute name="expr">_event.data.<xsl:value-of select="."/></xsl:attribute>
</xsl:template>

<!-- the value of a KVP specified by namelist from _event.data -->
<xsl:template match="//@conf:eventDataNamelistValue">
	<xsl:attribute name="expr">_event.data.Var<xsl:value-of select="."/></xsl:attribute>
</xsl:template>

<!-- the location of the scxml event i/o processor -->
<xsl:template match="//@conf:scxmlEventIOLocation">
	<xsl:attribute name="expr">_ioprocessors['http://www.w3.org/TR/scxml/#SCXMLEventProcessor'].location</xsl:attribute>
</xsl:template>

<!-- the expr versions of the send attributes take the value of the specified variable -->

<xsl:template match="//@conf:eventExpr">
	<xsl:attribute name="eventexpr">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:targetExpr">
	<xsl:attribute name="targetexpr">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:typeExpr">
	<xsl:attribute name="typeexpr">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:delayFromVar">
	<xsl:attribute name="delayexpr">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- a delayexpr computed from the value passed in -->
<xsl:template match="//@conf:delay">
	<xsl:attribute name="delayexpr">'<xsl:value-of select="."/>s'</xsl:attribute>
</xsl:template>

<!-- the specified variable is used as idlocation (a location) -->
<xsl:template match="//@conf:idlocation">
	<xsl:attribute name="idlocation">Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- the specified variable is used as sendidexpr -->
<xsl:template match="//@conf:sendIDExpr">
	<xsl:attribute name="sendidexpr">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- the specified variable is used as srcexpr -->
<xsl:template match="//@conf:srcExpr">
	<xsl:attribute name="srcexpr">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- the specified variable is used as namelist (data item names) -->
<xsl:template match="//@conf:namelist">
	<xsl:attribute name="namelist">Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- a reference to an invalid namelist, i.e. one that should cause an error -->
<xsl:template match="//@conf:invalidNamelist">
	<xsl:attribute name="namelist">&#34;foo</xsl:attribute>
</xsl:template>

<!-- transition conditions -->

<!-- the variable compared with a value that is evaluated -->
<xsl:template match="//@conf:idVal">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)([=&lt;&gt;]=?)(.*)">
					<xsl:matching-substring>ctx.Var<xsl:value-of select="regex-group(1)"/>
						<xsl:variable name="op"><xsl:value-of select="regex-group(2)"/></xsl:variable>
						<xsl:choose>
							<xsl:when test="$op='='"> == </xsl:when>
							<xsl:otherwise><xsl:value-of select="concat(' ', $op, ' ')"/></xsl:otherwise>
					 </xsl:choose>
					 	<xsl:value-of select="regex-group(3)"/>
					</xsl:matching-substring>
		</xsl:analyze-string>
	</xsl:attribute>
</xsl:template>

<!-- compare two variables -->
<xsl:template match="//@conf:varIdVal">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)([=&lt;&gt;]=?)(.*)">
					<xsl:matching-substring>ctx.Var<xsl:value-of select="regex-group(1)"/>
						<xsl:variable name="op"><xsl:value-of select="regex-group(2)"/></xsl:variable>
						<xsl:choose>
							<xsl:when test="$op='='"> == </xsl:when>
							<xsl:otherwise><xsl:value-of select="concat(' ', $op, ' ')"/></xsl:otherwise>
					 </xsl:choose>ctx.Var<xsl:value-of select="regex-group(3)"/>
					</xsl:matching-substring>
		</xsl:analyze-string>
	</xsl:attribute>
</xsl:template>

<!-- a var whose value was passed in via namelist has the specified value -->
<xsl:template match="//@conf:namelistIdVal">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)([=&lt;&gt;]=?)(.*)">
					<xsl:matching-substring>ctx.Var<xsl:value-of select="regex-group(1)"/>
						<xsl:variable name="op"><xsl:value-of select="regex-group(2)"/></xsl:variable>
						<xsl:choose>
							<xsl:when test="$op='='"> == </xsl:when>
							<xsl:otherwise><xsl:value-of select="concat(' ', $op, ' ')"/></xsl:otherwise>
					 </xsl:choose>
							<xsl:value-of select="regex-group(3)"/>
					</xsl:matching-substring>
		</xsl:analyze-string>
	</xsl:attribute>
</xsl:template>

<!-- true if the two vars have the same value -->
<xsl:template match="//@conf:VarEqVar">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)(\W+)([0-9]+)">
					<xsl:matching-substring>ctx.Var<xsl:value-of select="regex-group(1)"/> == ctx.Var<xsl:value-of select="regex-group(3)"/>
					</xsl:matching-substring>
		</xsl:analyze-string>
	</xsl:attribute>
</xsl:template>

<!-- true if the two vars have the same value, which is a structure -->
<xsl:template match="//@conf:VarEqVarStruct">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)(\W+)([0-9]+)">
					<xsl:matching-substring>ctx.Var<xsl:value-of select="regex-group(1)"/> == ctx.Var<xsl:value-of select="regex-group(3)"/>
					</xsl:matching-substring>
		</xsl:analyze-string>
	</xsl:attribute>
</xsl:template>

<!-- the variable compared with a quoted value -->
<xsl:template match="//@conf:idQuoteVal">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)([=&lt;&gt;]=?)(.*)">
<xsl:matching-substring>ctx.Var<xsl:value-of select="regex-group(1)"/><xsl:variable name="op"><xsl:value-of select="regex-group(2)"/></xsl:variable>
<xsl:choose>
<xsl:when test="$op='='"> == </xsl:when>
<xsl:otherwise><xsl:value-of select="concat(' ', $op, ' ')"/></xsl:otherwise></xsl:choose>'<xsl:value-of select="regex-group(3)"/>'</xsl:matching-substring>
</xsl:analyze-string>
</xsl:attribute>
</xsl:template>

<!-- compare the values of two vars -->
<xsl:template match="//@conf:compareIDVal">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)([=&lt;&gt;]=?)([0-9+])">
					<xsl:matching-substring>ctx.Var<xsl:value-of select="regex-group(1)"/>
						<xsl:variable name="op"><xsl:value-of select="regex-group(2)"/></xsl:variable>
						<xsl:choose>
							<xsl:when test="$op='='"> == </xsl:when>
							<xsl:otherwise><xsl:value-of select="concat(' ', $op, ' ')"/></xsl:otherwise>
					 </xsl:choose>ctx.Var<xsl:value-of select="regex-group(3)"/>
					</xsl:matching-substring>
		</xsl:analyze-string>
	</xsl:attribute>
</xsl:template>

<!-- the specified var has the value of <conf:someInlineVal> -->
<xsl:template match="//@conf:idSomeVal">
	<xsl:attribute name="cond">ctx.Var<xsl:value-of select="." /> == 123</xsl:attribute>
</xsl:template>

<!-- the event's name field has the specified value -->
<xsl:template match="//@conf:eventNameVal">
		<xsl:attribute name="cond">_event.name == <xsl:text>'</xsl:text><xsl:value-of select="."/><xsl:text>'</xsl:text>
	</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:eventvarVal">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)([=&lt;&gt;]=?)(.*)">
					<xsl:matching-substring>_event.data['Var<xsl:value-of select="regex-group(1)"/><xsl:text>']</xsl:text>
						<xsl:variable name="op"><xsl:value-of select="regex-group(2)"/></xsl:variable>
						<xsl:choose>
							<xsl:when test="$op='='"> == </xsl:when>
							<xsl:otherwise><xsl:value-of select="concat(' ', $op, ' ')"/></xsl:otherwise>
					 </xsl:choose>
							<xsl:value-of select="regex-group(3)"/>
					</xsl:matching-substring>
		</xsl:analyze-string>
	</xsl:attribute>
</xsl:template>

<!-- the variable matches the value of a system var -->
<xsl:template match="//@conf:idSystemVarVal">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)([=&lt;&gt;]=?)(.*)">
<xsl:matching-substring>ctx.Var<xsl:value-of select="regex-group(1)"/><xsl:variable name="op"><xsl:value-of select="regex-group(2)"/></xsl:variable>
<xsl:choose>
<xsl:when test="$op='='"> == </xsl:when>
<xsl:otherwise><xsl:value-of select="concat(' ', $op, ' ')"/></xsl:otherwise></xsl:choose><xsl:value-of select="regex-group(3)"/></xsl:matching-substring>
</xsl:analyze-string>
</xsl:attribute>
</xsl:template>

<!-- _event.data matches the specified value -->
<xsl:template match="//@conf:eventdataVal">
	<xsl:attribute name="cond">_event.data == <xsl:value-of select="."/></xsl:attribute>
</xsl:template>

<!-- _event.data is the value of <conf:someInlineVal> -->
<xsl:template match="//@conf:eventdataSomeVal">
	<xsl:attribute name="cond">_event.data == 123</xsl:attribute>
</xsl:template>

<!-- _event.data has no value -->
<xsl:template match="//@conf:emptyEventData">
	<xsl:attribute name="cond">_event.data == null</xsl:attribute>
</xsl:template>

<!-- the _name system var has the specified quoted value -->
<xsl:template match="//@conf:nameVarVal">
	<xsl:attribute name="cond">_name == '<xsl:value-of select="."/>'</xsl:attribute>
</xsl:template>

<!-- the first var's value is a prefix of the second var's value; input "n m" -->
<xsl:template match="//@conf:varPrefix">
			<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="(\w+)(\W)(\w+)">
					<xsl:matching-substring>string(ctx.Var<xsl:value-of select="regex-group(3)"/>).startsWith(string(ctx.Var<xsl:value-of select="regex-group(1)"/>))</xsl:matching-substring>
		</xsl:analyze-string>
	</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:inState">
	<xsl:attribute name="cond">In('<xsl:value-of select="."/>')</xsl:attribute>
</xsl:template>

<!-- a value that is not a boolean (a CEL guard must have type bool) -->
<xsl:template match="//@conf:nonBoolean">
	<xsl:attribute name="cond">'not a boolean'</xsl:attribute>
</xsl:template>

<!-- true if the variable has a value -->
<xsl:template match="//@conf:isBound">
	<xsl:attribute name="cond">ctx.Var<xsl:value-of select="." /> != null</xsl:attribute>
</xsl:template>

<!-- true if the variable has been created but is not bound -->
<xsl:template match="//@conf:unboundVar">
	<xsl:attribute name="cond">ctx.Var<xsl:value-of select="." /> == null</xsl:attribute>
</xsl:template>

<!-- true if the system var has a value -->
<xsl:template match="//@conf:systemVarIsBound">
	<xsl:attribute name="cond"><xsl:value-of select="." /> != null</xsl:attribute>
</xsl:template>

<!-- true if the variable does not have a value -->
<xsl:template match="//@conf:noValue">
	<xsl:attribute name="cond">ctx.Var<xsl:value-of select="." /> == null</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:true">
	<xsl:attribute name="cond">true</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:false">
	<xsl:attribute name="cond">false</xsl:attribute>
</xsl:template>

<!-- all the required fields of _event are present -->
  <xsl:template match="//@conf:eventFieldsAreBound">
    <xsl:attribute name="cond">has(_event.name) &amp;&amp; has(_event.type) &amp;&amp; has(_event.sendid) &amp;&amp; has(_event.origin) &amp;&amp; has(_event.origintype) &amp;&amp; has(_event.invokeid) &amp;&amp; has(_event.data)</xsl:attribute>
  </xsl:template>

<!-- _event.data contains the specified item -->
<xsl:template match="//@conf:eventDataHasField">
	<xsl:attribute name="cond">has(_event.data.<xsl:value-of select="."/>)</xsl:attribute>
</xsl:template>

<!-- the specified field of _event has no value -->
<xsl:template match="//@conf:eventFieldHasNoValue">
	<xsl:attribute name="cond">_event.<xsl:value-of select="." /> == null</xsl:attribute>
</xsl:template>

<!-- the language of _event matches the processor's datamodel -->
<xsl:template match="//@conf:eventLanguageMatchesDatamodel">
	<xsl:attribute name="cond">_event.language == 'cel'</xsl:attribute>
</xsl:template>

<!-- _event was delivered on the specified i/o processor -->
<xsl:template match="//@conf:originTypeEq">
	<xsl:attribute name="cond">_event.origintype == '<xsl:value-of select="."/>'</xsl:attribute>
</xsl:template>

<!-- scripting: CEL has no <script>; tests using these are not generated -->

<xsl:template match="conf:script">
 <script xmlns="http://www.w3.org/2005/07/scxml">var Var1 = 1</script>
</xsl:template>

<xsl:template match="//@conf:scriptGoodSrc">
	<xsl:attribute name="src">D:\W3C\SCXMLTests\test300.js</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:scriptBadSrc">
	<xsl:attribute name="src">D:\foo</xsl:attribute>
</xsl:template>

<!-- sends an event back to the sender of the current event -->
<xsl:template match="conf:sendToSender">
 <send xmlns="http://www.w3.org/2005/07/scxml">
 	<xsl:attribute name="event"><xsl:value-of select="@name" /></xsl:attribute>
   <xsl:attribute name="targetexpr">_event.origin</xsl:attribute>
   <xsl:attribute name="typeexpr">_event.origintype</xsl:attribute>
   </send>
</xsl:template>

<!-- foreach -->
<!-- a list containing 1 2 3 in that order (JSON content, which CEL reads as a list) -->
<xsl:template match="//conf:array123">[1,2,3]</xsl:template>

<!-- the value of the indicated variable as the 'array' attribute -->
<xsl:template match="//@conf:arrayVar">
	<xsl:attribute name="array">ctx.Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:arrayTextVar">
	<xsl:attribute name="array">ctx.Var<xsl:value-of select="."/></xsl:attribute>
</xsl:template>

<!-- an expr that yields a list containing 1 2 3 in that order -->
<xsl:template match="//@conf:arrayExpr123">
	<xsl:attribute name="expr">[1, 2, 3]</xsl:attribute>
</xsl:template>

<!-- an expr that evaluates to something that is not a list -->
<xsl:template match="//@conf:illegalArray">
		<xsl:attribute name="expr">7</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:item">
	<xsl:attribute name="item">Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- something that cannot be a variable name -->
<xsl:template match="//@conf:illegalItem">
	<xsl:attribute name="item">'continue'</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:index">
	<xsl:attribute name="index">Var<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<!-- add an extra item onto the end of the specified list -->
<xsl:template match="conf:extendArray">
	<assign xmlns="http://www.w3.org/2005/07/scxml">
	  <xsl:attribute name="location">Var<xsl:value-of select="@id"/></xsl:attribute>
	  <xsl:attribute name="expr">ctx.Var<xsl:value-of select="@id"/> + [4]</xsl:attribute>
	  </assign>
	</xsl:template>

<!-- a two-dimensional list all of whose cells are the specified value (not used by any test) -->
<xsl:template match="//@conf:multiDimensionalArrayExpr">
		<xsl:attribute name="expr">[[<xsl:value-of select="."/>,<xsl:value-of select="."/>],[<xsl:value-of select="."/>,<xsl:value-of select="."/>]]</xsl:attribute>
</xsl:template>

<!-- true iff each cell of the two-dimensional list has the value (not used by any test) -->
<xsl:template match="//@conf:arrayVal">
		<xsl:attribute name="cond">
		<xsl:analyze-string select="."
			regex="([0-9]+)(\W+)(.*)">
					<xsl:matching-substring>
				    <xsl:variable name="targetArray">ctx.Var<xsl:value-of select="regex-group(1)"/></xsl:variable>
					<xsl:variable name="value"><xsl:value-of select="regex-group(3)"/></xsl:variable>
<xsl:value-of select="$targetArray"/>[0][0] == <xsl:value-of select="$value"/> &amp;&amp; <xsl:value-of select="$targetArray"/>[0][1] == <xsl:value-of select="$value"/> &amp;&amp; <xsl:value-of select="$targetArray"/>[1][0] == <xsl:value-of select="$value"/> &amp;&amp; <xsl:value-of select="$targetArray"/>[1][1] == <xsl:value-of select="$value"/>
</xsl:matching-substring>
		</xsl:analyze-string>
	</xsl:attribute>
</xsl:template>

<!-- BASIC HTTP EVENT I/O PROCESSOR (site specific) -->

<xsl:template match="//@conf:testOnServer">
 <xsl:attribute name="target">SITE_SPECIFIC_ADDRESS<xsl:value-of select="." /></xsl:attribute>
</xsl:template>

<xsl:template match="conf:setUpHTTPTest">
</xsl:template>

<xsl:template match="//@conf:basicHTTPAccessURI">
 <xsl:attribute name="expr">_ioprocessors['basichttp']['location']</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:basicHTTPAccessURITarget">
 <xsl:attribute name="targetexpr">_ioprocessors['basichttp']['location']</xsl:attribute>
</xsl:template>

<!-- a namelist attribute containing all the ids listed in the attribute's value -->
<xsl:template match="//@conf:multipleNamelist">
 <xsl:attribute name="namelist">
 <xsl:analyze-string select="." regex="([0-9]+)">
<xsl:matching-substring>Var<xsl:value-of select="regex-group(1)"/><xsl:text>&#xa;</xsl:text></xsl:matching-substring>
</xsl:analyze-string>
</xsl:attribute>
</xsl:template>

<!-- true if the event is external -->
<xsl:template match="//@conf:eventIsExternal">
 <xsl:attribute name="cond">_event.type == 'external'</xsl:attribute>
</xsl:template>

<!-- The following Basic HTTP templates match ECMAScript regular expressions against
     _event.raw, which the CEL event does not have; tests using them are not generated. -->

<xsl:template match="//@conf:methodIsPost">
 <xsl:attribute name="cond">_event.raw.contains('POST')</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:eventIdParamHasValue">
 <xsl:attribute name="cond"><xsl:analyze-string select="." regex="(\S+)(\s+)(\S+)">
<xsl:matching-substring>_event.raw.contains('Var<xsl:value-of select="regex-group(1)"/>=<xsl:value-of select="regex-group(3)"/>')</xsl:matching-substring></xsl:analyze-string></xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:eventNamedParamHasValue">
 <xsl:attribute name="cond"><xsl:analyze-string select="." regex="(\S+)(\s+)(\S+)">
 <xsl:matching-substring>_event.raw.contains('<xsl:value-of select="regex-group(1)"/>=<xsl:value-of select="regex-group(3)"/>')</xsl:matching-substring></xsl:analyze-string></xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:messageBodyEquals">
 <xsl:attribute name="cond">_event.raw.matches('<xsl:value-of select="."/>')</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:getNamedParamVal">
 <xsl:attribute name="expr">_event.raw</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:getIDParamVal">
 <xsl:attribute name="expr">_event.raw</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:getNthParamName">
 <xsl:attribute name="expr">_event.raw</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:getNthParamVal">
 <xsl:attribute name="expr">_event.raw</xsl:attribute>
</xsl:template>

<xsl:template match="//@conf:scxmlEventExpr">
 <xsl:attribute name="expr">_event.raw</xsl:attribute>
 </xsl:template>

<xsl:template match="conf:msgContent">
<content xmlns="http://www.w3.org/2005/07/scxml">
	<xsl:attribute name="expr">'address=' + _ioprocessors['basichttp']['location']</xsl:attribute>
	</content>
</xsl:template>

<xsl:template match="//@conf:msgIsBody">
 <xsl:attribute name="cond">_event.raw.contains('address=')</xsl:attribute>
</xsl:template>
</xsl:stylesheet>
